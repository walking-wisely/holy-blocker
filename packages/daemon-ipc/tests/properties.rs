use daemon_ipc::{ClientRole, End, Event, Limits, Session, frame};
use proptest::prelude::*;

const MAX: u32 = 256;

fn stream() -> Vec<u8> {
    let mut out = Vec::new();
    for body in [
        r#"{"type":"hello","proto":1}"#,
        r#"{"type":"list_apps"}"#,
        r#"{"type":"future","a":[1,2,3]}"#,
        r#"{"type":"request_add_app","identity":"com.example.app"}"#,
        r#"{"type":"cancel_disarm"}"#,
    ] {
        out.extend(frame::encode(body.as_bytes(), Limits::new(MAX)).unwrap());
    }
    out
}

fn run(chunks: &[&[u8]]) -> Vec<Event> {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(MAX));
    chunks.iter().flat_map(|c| session.receive(c)).collect()
}

proptest! {
    #[test]
    fn any_chunking_yields_the_same_events(cuts in proptest::collection::vec(0usize..200, 0..12)) {
        let bytes = stream();
        let whole = run(&[&bytes]);
        let mut points: Vec<usize> = cuts.into_iter().map(|c| c % (bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut pieces: Vec<&[u8]> = Vec::new();
        let mut at = 0;
        for p in points {
            pieces.push(&bytes[at..p]);
            at = p;
        }
        pieces.push(&bytes[at..]);
        prop_assert_eq!(run(&pieces), whole);
    }

    #[test]
    fn arbitrary_bytes_never_panic_or_over_buffer(
        data in proptest::collection::vec(any::<u8>(), 0..4096),
        step in 1usize..64,
    ) {
        let mut session = Session::new(End::Daemon(ClientRole::Agent), Limits::new(MAX));
        for piece in data.chunks(step) {
            session.receive(piece);
            prop_assert!(session.buffered() <= frame::LENGTH_PREFIX + MAX as usize);
        }
    }

    #[test]
    fn arbitrary_bodies_after_hello_never_panic(body in proptest::collection::vec(any::<u8>(), 0..200)) {
        let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(MAX));
        let mut input = frame::encode(br#"{"type":"hello","proto":1}"#, Limits::new(MAX)).unwrap();
        input.extend(frame::encode(&body, Limits::new(MAX)).unwrap());
        session.receive(&input);
        prop_assert!(session.buffered() <= frame::LENGTH_PREFIX + MAX as usize);
    }

    #[test]
    fn one_huge_read_still_buffers_at_most_one_frame(declared in 0u32..2_000_000) {
        let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(MAX));
        let mut input = declared.to_be_bytes().to_vec();
        input.resize(4 + 100_000, b'x');
        session.receive(&input);
        prop_assert!(session.buffered() <= frame::LENGTH_PREFIX + MAX as usize);
    }
}
