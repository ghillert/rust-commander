//! Decode directory-index links and their server-provided display metadata.
//! Listing sizes may be rounded; operations still stat files for authoritative values.
use anyhow::{Result, ensure};
use chrono::{DateTime, NaiveDateTime, Utc};
use opendal::{EntryMode, Metadata, raw::oio};
use scraper::{ElementRef, Html, Selector};
use std::collections::{BTreeMap, HashMap};
use url::Url;

pub(super) fn parse(
    bytes: &[u8],
    base: &Url,
    path: &str,
    links: &mut HashMap<String, Url>,
) -> Result<Vec<oio::Entry>> {
    let doc = Html::parse_document(&String::from_utf8_lossy(bytes));
    let headings = Selector::parse("title,h1").unwrap();
    let heading = doc
        .select(&headings)
        .flat_map(|e| e.text())
        .collect::<String>()
        .to_ascii_lowercase();
    let caddy = Selector::parse("table.entries, table#files, .file-listing, .listing").unwrap();
    ensure!(
        heading.contains("index of")
            || heading.contains("directory listing")
            || heading.contains("directory of")
            || doc.select(&caddy).next().is_some(),
        "This website is not a directory listing. Open it in your browser, or paste a direct download URI."
    );
    let selector = Selector::parse("a[href]").unwrap();
    let time = Selector::parse("time[datetime]").unwrap();
    let parent = base.path().trim_end_matches('/').to_owned() + "/";
    let prefix = path.trim_matches('/');
    let mut entries = BTreeMap::new();
    for anchor in doc.select(&selector) {
        let href = anchor.value().attr("href").unwrap_or_default();
        if href.starts_with(['?', '#']) {
            continue;
        }
        let Ok(url) = base.join(href) else {
            continue;
        };
        if url.origin() != base.origin() || !url.username().is_empty() || url.password().is_some() {
            continue;
        }
        let Some(child) = url.path().strip_prefix(&parent) else {
            continue;
        };
        let directory = child.ends_with('/');
        let child = child.trim_end_matches('/');
        if child.is_empty() || child.contains('/') {
            continue;
        }
        let Ok(name) = percent_encoding::percent_decode_str(child).decode_utf8() else {
            continue;
        };
        if crate::filesystem::validate_name(&name).is_err() || name.contains(':') {
            continue;
        }
        let key = if prefix.is_empty() {
            name.into_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let mut meta = Metadata::new(if directory {
            EntryMode::DIR
        } else {
            EntryMode::FILE
        });
        let details = listing_metadata(anchor, &time);
        if let Some(size) = details.size.filter(|_| !directory) {
            meta.set_content_length(size);
        } else {
            meta = meta.with_user_metadata(HashMap::from([(
                "rust-commander-size-known".into(),
                "false".into(),
            )]));
        }
        if let Some(modified) = details.modified {
            meta.set_last_modified(modified);
        }
        links.insert(key.clone(), url);
        entries.insert(
            key.clone(),
            oio::Entry::new(&(key + if directory { "/" } else { "" }), meta),
        );
    }
    Ok(entries.into_values().collect())
}

#[derive(Default)]
struct ListingMetadata {
    size: Option<u64>,
    modified: Option<DateTime<Utc>>,
}

fn listing_metadata(anchor: ElementRef<'_>, time: &Selector) -> ListingMetadata {
    if let Some(row) = anchor
        .ancestors()
        .filter_map(ElementRef::wrap)
        .find(|e| e.value().name() == "tr")
    {
        let name_cell = anchor
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|e| matches!(e.value().name(), "td" | "th"));
        let mut result = ListingMetadata::default();
        // Apache: name, modified, size, description. Caddy: name, size, modified.
        // Limit this to the two metadata cells so description text is never used.
        for cell in row
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|e| matches!(e.value().name(), "td" | "th"))
            .skip_while(|e| Some(e.id()) != name_cell.map(|e| e.id()))
            .skip(1)
            .take(2)
        {
            let text = cell.text().collect::<String>();
            // Caddy exposes exact bytes even when its visible size is rounded.
            let size = cell
                .value()
                .attr("data-size")
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| parse_size(&text));
            result.size = result.size.or(size);
            result.modified = result.modified.or_else(|| {
                cell.select(time)
                    .find_map(|e| e.value().attr("datetime").and_then(parse_modified))
                    .or_else(|| parse_modified(&text))
            });
        }
        return result;
    }
    // Apache and nginx preformatted indexes put date and size after each link.
    // Stop at the row boundary: metadata must never leak from a following file.
    let mut line = String::new();
    for sibling in anchor.next_siblings() {
        let Some(text) = sibling.value().as_text() else {
            if sibling.value().is_comment() {
                continue;
            }
            break;
        };
        if let Some((first, _)) = text.split_once(['\n', '\r']) {
            line.push_str(first);
            break;
        }
        line.push_str(text);
    }
    let words: Vec<_> = line.split_whitespace().collect();
    let mut result = ListingMetadata::default();
    if words.len() >= 2 {
        result.modified = parse_modified(&format!("{} {}", words[0], words[1]));
        // A unit may be separated by whitespace (e.g. 1.5 MiB).
        result.size = words.get(2).and_then(|size| {
            words
                .get(3)
                .and_then(|unit| parse_size(&format!("{size} {unit}")))
                .or_else(|| parse_size(size))
        });
    }
    result
}

fn parse_modified(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.to_utc());
    }
    if let Ok(date) = DateTime::parse_from_rfc2822(text) {
        return Some(date.to_utc());
    }
    // Indexes with no timezone are interpreted as UTC, like nginx's default.
    // Unlike a HEAD response, the HTML does not tell us the server's timezone.
    for format in [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%d-%b-%Y %H:%M",
        "%d-%b-%Y %H:%M:%S",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(text, format) {
            return Some(date.and_utc());
        }
    }
    None
}

fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let end = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let number = &text[..end];
    let unit = text[end..].trim().to_ascii_lowercase();
    let multiplier: u64 = match unit.as_str() {
        "" | "b" => 1,
        "k" | "ki" | "kib" => 1024,
        "m" | "mi" | "mib" => 1024_u64.pow(2),
        "g" | "gi" | "gib" => 1024_u64.pow(3),
        "t" | "ti" | "tib" => 1024_u64.pow(4),
        "p" | "pi" | "pib" => 1024_u64.pow(5),
        "e" | "ei" | "eib" => 1024_u64.pow(6),
        "kb" => 1000,
        "mb" => 1000_u64.pow(2),
        "gb" => 1000_u64.pow(3),
        "tb" => 1000_u64.pow(4),
        "pb" => 1000_u64.pow(5),
        "eb" => 1000_u64.pow(6),
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    // Sizes without units must be whole byte counts. Use checked integer
    // arithmetic so huge or malformed values stay unknown instead of wrapping.
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (number.contains('.') && (fraction.is_empty() || multiplier == 1))
    {
        return None;
    }
    let whole = whole.parse::<u64>().ok()? as u128;
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        let divisor = 10_u128.checked_pow(fraction.len().try_into().ok()?)?;
        fraction
            .parse::<u128>()
            .ok()?
            .checked_mul(multiplier as u128)?
            / divisor
    };
    u64::try_from(
        whole
            .checked_mul(multiplier as u128)?
            .checked_add(fraction_value)?,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_filter_and_decode_children() {
        let html=br#"<title>Index of /dir/</title><a href='../'>Parent</a><a href='?C=N'>sort</a><a href='a%20b.pdf?token=secret'>a</a><a href='folder/'>f</a><a href='https://elsewhere.test/dir/evil'>x</a><a href='deep/nested.txt'>n</a><a href='%2e%2e/'>bad</a><a href='javascript:alert(1)'>bad</a>"#;
        let mut links = HashMap::new();
        let entries = parse(
            html,
            &Url::parse("https://example.test/dir/").unwrap(),
            "dir",
            &mut links,
        )
        .unwrap();
        assert_eq!(
            entries.iter().map(|e| e.path()).collect::<Vec<_>>(),
            ["dir/a b.pdf", "dir/folder/"]
        );
        assert_eq!(links["dir/a b.pdf"].query(), Some("token=secret"));
    }
    #[test]
    fn empty_index_and_ordinary_website() {
        let base = Url::parse("https://example.test/").unwrap();
        assert!(
            parse(b"<h1>Index of /</h1>", &base, "", &mut HashMap::new())
                .unwrap()
                .is_empty()
        );
        assert!(
            parse(
                b"<h1>Welcome</h1><a href='login'>Login</a>",
                &base,
                "",
                &mut HashMap::new()
            )
            .is_err()
        );
    }

    fn listing(html: &str) -> Vec<oio::Entry> {
        parse(
            html.as_bytes(),
            &Url::parse("https://example.test/dir/").unwrap(),
            "dir",
            &mut HashMap::new(),
        )
        .unwrap()
    }
    fn check(entry: &oio::Entry, size: Option<u64>, modified: Option<&str>) {
        let mut expected = Metadata::new(entry.mode());
        if let Some(size) = size {
            expected.set_content_length(size);
        } else {
            expected = expected.with_user_metadata(HashMap::from([(
                "rust-commander-size-known".into(),
                "false".into(),
            )]));
        }
        if let Some(modified) = modified {
            expected.set_last_modified(DateTime::parse_from_rfc3339(modified).unwrap().to_utc());
        }
        assert_eq!(entry, &oio::Entry::new(entry.path(), expected));
    }
    #[test]
    fn apache_preformatted_metadata_matches_kew() {
        let entries = listing(
            r#"<h1>Index of /dir/</h1><pre>
<a href="Archive/">Archive/</a>                   2026-06-04 15:30    -
<a href="Special_Issue_28_Feb_2022/">Special_Issue_28_Feb_2022/</a> 2022-11-09 15:20    -
<a href="wcvp.zip">wcvp.zip</a>                   2026-06-04 15:30   84M
<a href="wcvp_dwca.zip">wcvp_dwca.zip</a>              2026-06-04 15:31   84M
<hr></pre>"#,
        );
        assert_eq!(entries.len(), 4);
        check(&entries[0], None, Some("2026-06-04T15:30:00Z"));
        check(&entries[1], None, Some("2022-11-09T15:20:00Z"));
        check(
            &entries[2],
            Some(84 * 1024 * 1024),
            Some("2026-06-04T15:30:00Z"),
        );
        check(
            &entries[3],
            Some(84 * 1024 * 1024),
            Some("2026-06-04T15:31:00Z"),
        );
    }
    #[test]
    fn nginx_preformatted_exact_and_human_sizes() {
        let entries = listing(
            r#"<title>Index of /dir/</title><pre>
<a href="a.txt">a.txt</a> 04-Jun-2026 15:30 123456
<a href="b.txt">b.txt</a> 04-Jun-2026 15:31 1.5K
<a href="empty.txt">empty.txt</a> 04-Jun-2026 15:32 0
</pre>"#,
        );
        check(&entries[0], Some(123456), Some("2026-06-04T15:30:00Z"));
        check(&entries[1], Some(1536), Some("2026-06-04T15:31:00Z"));
        check(&entries[2], Some(0), Some("2026-06-04T15:32:00Z"));
    }
    #[test]
    fn apache_table_keeps_metadata_with_its_row() {
        let entries = listing(
            r#"<h1>Index of /dir/</h1><table>
<tr><th></th><th>Name</th><th>Last modified</th><th>Size</th><th>Description</th></tr>
<tr><td><img src="/icons/file.svg"></td><td><a href="a.txt">a.txt</a></td><td>2026-06-04 15:30</td><td>1.5K</td><td>9999</td></tr>
<tr><td></td><td><a href="b.txt">b.txt</a></td><td>-</td><td>-</td><td>9999</td></tr>
<tr><td></td><td><a href="c.txt">c.txt</a></td><td>2026-06-05 16:31:32</td><td>0</td><td></td></tr>
</table>"#,
        );
        check(&entries[0], Some(1536), Some("2026-06-04T15:30:00Z"));
        check(&entries[1], None, None);
        check(&entries[2], Some(0), Some("2026-06-05T16:31:32Z"));
    }
    #[test]
    fn caddy_prefers_exact_bytes_and_timezone_aware_time() {
        let entries = listing(
            r#"<div class="listing"><table>
<tr><td></td><td><a href="a.txt"><span class="name">a.txt</span></a></td>
<td class="size" data-size="1234567"><div>1.2 MB</div></td>
<td class="timestamp"><time datetime="2026-06-04T15:30:12+02:00">Jun 4, 2026</time></td><td></td></tr>
<tr><td></td><td><a href="folder/">folder/</a></td><td>&mdash;</td>
<td><time datetime="2026-06-04T13:30:12Z">just now</time></td><td></td></tr>
</table></div>"#,
        );
        check(&entries[0], Some(1234567), Some("2026-06-04T13:30:12Z"));
        check(&entries[1], None, Some("2026-06-04T13:30:12Z"));
    }
    #[test]
    fn missing_or_invalid_metadata_stays_unknown_without_borrowing_other_rows() {
        let entries = listing(
            r#"<title>Index of /dir/</title><pre>
<a href="a.txt">2026-06-04 15:30 1000</a>
<a href="b.txt">b.txt</a> no metadata
<a href="c.txt">c.txt</a> 2026-06-04 15:30 -
<a href="d.txt">d.txt</a> 2026-06-04 15:30 -1
<a href="e.txt">e.txt</a> 2026-99-99 99:99 100
</pre>"#,
        );
        check(&entries[0], None, None);
        check(&entries[1], None, None);
        check(&entries[2], None, Some("2026-06-04T15:30:00Z"));
        check(&entries[3], None, Some("2026-06-04T15:30:00Z"));
        check(&entries[4], Some(100), None);
    }
    #[test]
    fn size_units_are_checked_and_preserve_zero() {
        for (text, bytes) in [
            ("0", 0),
            ("123456", 123456),
            ("1.5K", 1536),
            ("84M", 84 * 1024 * 1024),
            ("2 G", 2 * 1024_u64.pow(3)),
            ("1.5 MiB", 1572864),
            ("1.2 MB", 1200000),
            (" 7 B ", 7),
            ("18446744073709551615", u64::MAX),
        ] {
            assert_eq!(parse_size(text), Some(bytes), "{text}");
        }
        for text in [
            "",
            "-",
            "—",
            "-1",
            "NaN",
            "1e3",
            "12 XY",
            "1.2.3K",
            "18446744073709551616",
            "18446744073709551615K",
            "0.5",
            "3.K",
        ] {
            assert_eq!(parse_size(text), None, "{text}");
        }
    }
}
