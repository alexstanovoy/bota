//! A frame is decoded into one message and nothing else, and a hostile frame of the largest size
//! makes a reader allocate little more than the frame itself.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bota_proto::{
    ClientMsg, CodecError, EventKind, FrameReader, Kit, MAX_DECODED_BYTES, MAX_PAYLOAD_LEN,
    ServerMsg, ShopEntry, Vec2, decode_payload, encode_frame_to_vec,
};

/// The bytes live now and the most that were live at once since the last reset.
struct Counting {
    live: AtomicUsize,
    peak: AtomicUsize,
}

#[global_allocator]
static COUNTING: Counting = Counting {
    live: AtomicUsize::new(0),
    peak: AtomicUsize::new(0),
};

// SAFETY: forwards every call to the system allocator and only counts the sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = self.live.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
        self.peak.fetch_max(live, Ordering::SeqCst);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.live.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            let live = self
                .live
                .fetch_add(new_size - layout.size(), Ordering::SeqCst)
                + (new_size - layout.size());
            self.peak.fetch_max(live, Ordering::SeqCst);
        } else {
            self.live
                .fetch_sub(layout.size() - new_size, Ordering::SeqCst);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// The allocator is the process's, so the tests that read it take turns.
static TURN: Mutex<()> = Mutex::new(());

/// What `read` allocated at its peak beyond what was live when it began.
fn peak_of<T>(read: impl FnOnce() -> T) -> (T, usize) {
    let before = COUNTING.live.load(Ordering::SeqCst);
    COUNTING.peak.store(before, Ordering::SeqCst);
    let out = read();
    let peak = COUNTING.peak.load(Ordering::SeqCst);
    (out, peak.saturating_sub(before))
}

fn varint(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A `Kit` whose item list claims `claimed` slots and holds that many empty ones: a byte each on
/// the wire and an `Option<ItemView>` each in memory.
fn kit_of_empty_slots(claimed: usize) -> Vec<u8> {
    let mut payload = vec![0]; // no abilities
    payload.extend(varint(claimed));
    payload.resize(payload.len() + claimed, 0); // each slot None
    payload.push(0); // no town portal
    payload
}

/// A `ShopEntry` whose alternatives claim `claimed` recipes and hold that many empty ones: a byte
/// each on the wire and a `Vec` header each in memory.
fn shop_entry_of_empty_recipes(claimed: usize) -> Vec<u8> {
    let mut payload = vec![1, 0, 0]; // item 1, cost 0, no components
    payload.extend(varint(claimed));
    payload.resize(payload.len() + claimed, 0);
    payload
}

#[test]
fn a_frame_of_the_largest_size_allocates_little_more_than_itself_whatever_it_claims() {
    let _turn = TURN.lock().unwrap();
    let hostile: [(&str, Vec<u8>); 2] = [
        ("items", kit_of_empty_slots(MAX_PAYLOAD_LEN - 8)),
        ("recipes", shop_entry_of_empty_recipes(MAX_PAYLOAD_LEN - 8)),
    ];
    for (name, payload) in hostile {
        assert!(payload.len() <= MAX_PAYLOAD_LEN);
        let (read, peak) = peak_of(|| match name {
            "items" => decode_payload::<Kit>(&payload).map(drop),
            _ => decode_payload::<ShopEntry>(&payload).map(drop),
        });
        assert_eq!(read, Err(CodecError::Malformed), "{name}");
        assert!(
            peak <= 64 * 1024,
            "{name}: {peak} bytes allocated reading it"
        );
    }
}

/// The most events a frame holds, each three bytes: the heaviest thing a frame makes a reader keep.
#[test]
fn the_most_events_a_frame_holds_are_read_within_the_decoded_bound() {
    let _turn = TURN.lock().unwrap();
    let event = EventKind::TreeGrew { at: Vec2::ZERO };
    let events = vec![event; (MAX_PAYLOAD_LEN - 16) / 3];
    let payload = postcard::to_allocvec(&ServerMsg::Events { tick: 1, events }).expect("a payload");
    assert!(payload.len() <= MAX_PAYLOAD_LEN, "{}", payload.len());
    let (read, peak) = peak_of(|| decode_payload::<ServerMsg>(&payload));
    assert!(matches!(read, Ok(ServerMsg::Events { .. })));
    assert!(
        peak <= MAX_DECODED_BYTES,
        "{peak} bytes to read {} of them",
        payload.len()
    );
}

#[test]
fn a_collection_at_its_bound_is_read_and_one_past_it_is_not() {
    let most = bota_proto::HERO_ITEM_SLOTS;
    assert!(decode_payload::<Kit>(&kit_of_empty_slots(most)).is_ok());
    assert_eq!(
        decode_payload::<Kit>(&kit_of_empty_slots(most + 1)),
        Err(CodecError::Malformed)
    );
    let most = bota_proto::MAX_SHOP_RECIPES;
    assert!(decode_payload::<ShopEntry>(&shop_entry_of_empty_recipes(most)).is_ok());
    assert_eq!(
        decode_payload::<ShopEntry>(&shop_entry_of_empty_recipes(most + 1)),
        Err(CodecError::Malformed)
    );
}

#[test]
fn a_frame_has_one_encoding_so_bytes_after_the_message_are_refused() {
    let frame = encode_frame_to_vec(&ServerMsg::Ping).expect("a frame");
    let payload = &frame[bota_proto::LEN_PREFIX..];
    assert!(matches!(
        decode_payload::<ServerMsg>(payload),
        Ok(ServerMsg::Ping)
    ));
    let mut padded = payload.to_vec();
    padded.push(0);
    assert_eq!(
        decode_payload::<ServerMsg>(&padded),
        Err(CodecError::Malformed)
    );

    let mut stream = FrameReader::new();
    let mut longer = frame.clone();
    longer.push(0);
    let length = (payload.len() + 1) as u32;
    longer[..bota_proto::LEN_PREFIX].copy_from_slice(&length.to_le_bytes());
    stream.push(&longer);
    assert_eq!(
        stream.next_message::<ServerMsg>(),
        Err(CodecError::Malformed)
    );

    let hello = ClientMsg::hello(bota_proto::Role::Bot, "bot", None, 0);
    let mut payload = postcard::to_allocvec(&hello).expect("a payload");
    payload.extend_from_slice(&[1, 2, 3]);
    assert_eq!(
        decode_payload::<ClientMsg>(&payload),
        Err(CodecError::Malformed)
    );
}
