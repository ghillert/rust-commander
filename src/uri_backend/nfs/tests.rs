use super::*;
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
};

fn request(socket: &mut TcpStream, program: u32, version: u32, procedure: u32) -> (u32, Vec<u8>) {
    let mut header = [0; 4];
    socket.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; (u32::from_be_bytes(header) & 0x7fff_ffff) as usize];
    socket.read_exact(&mut bytes).unwrap();
    let mut data = Decoder(&bytes);
    let xid = data.word().unwrap();
    for expected in [0, 2, program, version, procedure] {
        assert_eq!(data.word().unwrap(), expected);
    }
    for _ in 0..2 {
        data.word().unwrap(); // auth flavor
        data.opaque(400).unwrap();
    }
    (xid, data.0.to_vec())
}

fn reply(socket: &mut TcpStream, xid: u32, result: Xdr) {
    let mut reply = Xdr::default();
    for word in [xid, 1, 0, 0, 0, 0] {
        reply.word(word);
    }
    reply.0.extend(result.0);
    // Split a record across fragments to exercise the TCP record layer.
    socket.write_all(&12_u32.to_be_bytes()).unwrap();
    socket.write_all(&reply.0[..12]).unwrap();
    socket
        .write_all(&(0x8000_0000 | (reply.0.len() as u32 - 12)).to_be_bytes())
        .unwrap();
    socket.write_all(&reply.0[12..]).unwrap();
}

fn words(values: &[u32]) -> Xdr {
    let mut data = Xdr::default();
    for &word in values {
        data.word(word);
    }
    data
}

fn regular_file(data: &mut Xdr) {
    for word in [1, 0o644, 1, 1000, 1000] {
        data.word(word);
    }
    data.wide(5);
    data.0.extend([0; 40]);
    data.word(1_700_000_000);
    data.word(0);
    data.wide(0);
}

fn lookup(socket: &mut TcpStream) {
    let (xid, args) = request(socket, NFS, 3, 3);
    let mut args = Decoder(&args);
    assert_eq!(args.opaque(64).unwrap(), b"root");
    assert_eq!(args.opaque(255).unwrap(), b"test.txt");
    let mut result = words(&[0]);
    result.opaque(b"file");
    result.word(1);
    regular_file(&mut result);
    result.word(0);
    reply(socket, xid, result);
}

#[test]
fn opens_export_lists_with_fallback_and_reads_at_offset() {
    let mapper = TcpListener::bind("127.0.0.1:0").unwrap();
    let mount = TcpListener::bind("127.0.0.1:0").unwrap();
    let nfs = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = mapper.local_addr().unwrap();
    let fixture = std::thread::spawn(move || {
        let mut rpc = mapper.accept().unwrap().0;
        for (program, listener) in [(MOUNT, &mount), (NFS, &nfs)] {
            let (xid, args) = request(&mut rpc, PORTMAP, 2, 3);
            assert_eq!(args, words(&[program, 3, 6, 0]).0);
            reply(
                &mut rpc,
                xid,
                words(&[listener.local_addr().unwrap().port().into()]),
            );
        }

        let mut rpc = mount.accept().unwrap().0;
        let (xid, args) = request(&mut rpc, MOUNT, 3, 1);
        assert_eq!(Decoder(&args).opaque(1024).unwrap(), b"/export");
        let mut mounted = words(&[0]);
        mounted.opaque(b"root");
        mounted.0.extend(words(&[1, 1]).0);
        reply(&mut rpc, xid, mounted);

        let mut rpc = nfs.accept().unwrap().0;
        let (xid, _) = request(&mut rpc, NFS, 3, 17);
        reply(&mut rpc, xid, words(&[10004, 0]));
        let (xid, _) = request(&mut rpc, NFS, 3, 16);
        let mut listing = words(&[0, 0]);
        listing.wide(0);
        listing.word(1);
        listing.wide(2);
        listing.opaque(b"test.txt");
        listing.wide(3);
        listing.word(0);
        listing.word(1);
        reply(&mut rpc, xid, listing);
        lookup(&mut rpc);

        let mut rpc = nfs.accept().unwrap().0;
        lookup(&mut rpc);

        let mut rpc = nfs.accept().unwrap().0;
        lookup(&mut rpc);
        let (xid, args) = request(&mut rpc, NFS, 3, 6);
        let mut args = Decoder(&args);
        assert_eq!(args.opaque(64).unwrap(), b"file");
        assert_eq!(args.wide().unwrap(), 2);
        let mut data = words(&[0, 0, 3, 1]);
        data.opaque(b"llo");
        reply(&mut rpc, xid, data);
    });
    let export = Nfs::mount(address, None, "/export").unwrap();
    let entries = export.list("/").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path(), "test.txt");
    assert_eq!(export.stat("test.txt").unwrap().content_length(), 5);
    let mut bytes = Vec::new();
    export
        .read("test.txt", 2)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"llo");
    fixture.join().unwrap();
}

#[test]
fn rejects_excessive_or_truncated_xdr_fields() {
    let words = words(&[u32::MAX]);
    assert!(Decoder(&words.0).opaque(255).is_err());
    assert!(Decoder(&[0, 0, 0, 4, 0, 0]).opaque(255).is_err());
}

#[test]
fn lists_exports_via_rpcbind_and_ignores_invalid_paths() {
    let mapper = TcpListener::bind("127.0.0.1:0").unwrap();
    let mount = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = mapper.local_addr().unwrap();
    let fixture = std::thread::spawn(move || {
        let mut rpc = mapper.accept().unwrap().0;
        let (xid, args) = request(&mut rpc, PORTMAP, 2, 3);
        assert_eq!(args, words(&[MOUNT, 3, 6, 0]).0);
        reply(
            &mut rpc,
            xid,
            words(&[mount.local_addr().unwrap().port().into()]),
        );

        let mut rpc = mount.accept().unwrap().0;
        let (xid, args) = request(&mut rpc, MOUNT, 3, 5);
        assert!(args.is_empty());
        let mut exports = Xdr::default();
        for path in ["/media/music", "/backup", "/media/music", "/../invalid"] {
            exports.word(1);
            exports.opaque(path.as_bytes());
            exports.word(1);
            exports.opaque(b"some-clients");
            exports.word(0);
        }
        exports.word(0);
        assert!(decode_exports(&exports.0[..exports.0.len() - 1]).is_err());
        reply(&mut rpc, xid, exports);
    });
    assert_eq!(exports_at(address).unwrap(), ["/backup", "/media/music"]);
    fixture.join().unwrap();
}
