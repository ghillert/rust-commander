//! Visual validation hook; no settings or file-operation state is loaded.
use super::*;
use anyhow::{Context, Result};

pub(super) fn run(path: PathBuf) -> Result<()> {
    let ui = IconGallery::new()?;
    let dark = std::env::var("RUST_COMMANDER_ICON_GALLERY_SCHEME").is_ok_and(|v| v == "dark");
    ui.set_dark(dark);
    ui.global::<Chrome>().set_dark(dark);
    let ids = crate::icons::catalog_ids();
    ui.set_icons(strings(ids.iter().copied()));
    for id in ids {
        let icon = ui.global::<IconCatalog>().invoke_image((*id).into(), dark);
        let pixels = icon
            .to_rgba8()
            .with_context(|| format!("Cannot rasterize icon {id}"))?;
        anyhow::ensure!(
            pixels.as_slice().iter().any(|p| p.a > 0),
            "Transparent icon: {id}"
        );
    }
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_secs(1),
        move || {
            let snapshot = (|| -> Result<()> {
                let ui = weak.upgrade().context("Icon gallery closed early")?;
                let buffer = ui.window().take_snapshot()?;
                image::save_buffer(
                    &path,
                    buffer.as_bytes(),
                    buffer.width(),
                    buffer.height(),
                    image::ColorType::Rgba8,
                )?;
                Ok(())
            })();
            *output.borrow_mut() = snapshot.err().map(|e| format!("{e:#}"));
            let _ = slint::quit_event_loop();
        },
    );
    ui.run()?;
    if let Some(error) = result.borrow_mut().take() {
        anyhow::bail!(error);
    }
    println!("Rendered {} catalog icons", ids.len());
    Ok(())
}
