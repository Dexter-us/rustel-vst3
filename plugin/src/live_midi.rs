//! Local MIDI event bridge for the portable VST3 generator helper.
use crossbeam_queue::ArrayQueue;
use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const HEADER_BYTES: usize = 26;
const EVENT_BYTES: usize = 4;
const MAX_PACKET_EVENTS: usize = 64;
const QUEUE_CAPACITY: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiveMidiEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
}

pub struct LiveMidiBridge {
    port: u16,
    token: [u8; 16],
    events: Arc<ArrayQueue<LiveMidiEvent>>,
    dropped: Arc<AtomicU64>,
    last_packet_millis: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    receiver: Option<JoinHandle<()>>,
}

impl LiveMidiBridge {
    pub fn new() -> Result<Self, String> {
        let socket = UdpSocket::bind(("127.0.0.1", 0))
            .map_err(|error| format!("Could not open Rustel's local MIDI receiver: {error}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|error| format!("Could not configure Rustel's MIDI receiver: {error}"))?;
        let port = socket
            .local_addr()
            .map_err(|error| format!("Could not read Rustel's MIDI receiver port: {error}"))?
            .port();
        let token = make_token(port);
        let events = Arc::new(ArrayQueue::new(QUEUE_CAPACITY));
        let dropped = Arc::new(AtomicU64::new(0));
        let last_packet_millis = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));

        let receiver_socket = socket
            .try_clone()
            .map_err(|error| format!("Could not start Rustel's MIDI receiver: {error}"))?;
        let receiver_events = Arc::clone(&events);
        let receiver_dropped = Arc::clone(&dropped);
        let receiver_last_packet = Arc::clone(&last_packet_millis);
        let receiver_running = Arc::clone(&running);
        let receiver = thread::Builder::new()
            .name("rustel-live-midi".into())
            .spawn(move || {
                let mut packet = [0u8; HEADER_BYTES + EVENT_BYTES * MAX_PACKET_EVENTS];
                while receiver_running.load(Ordering::Acquire) {
                    let Ok((length, address)) = receiver_socket.recv_from(&mut packet) else {
                        continue;
                    };
                    if !is_loopback(address) {
                        continue;
                    }
                    let Some(decoded) = decode_packet(&packet[..length], &token) else {
                        continue;
                    };
                    receiver_last_packet.store(unix_millis(), Ordering::Release);
                    for event in decoded {
                        if receiver_events.push(event).is_err() {
                            receiver_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
            .map_err(|error| format!("Could not start Rustel's MIDI receiver: {error}"))?;

        Ok(Self {
            port,
            token,
            events,
            dropped,
            last_packet_millis,
            running,
            receiver: Some(receiver),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token_hex(&self) -> String {
        self.token
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn pop(&self) -> Option<LiveMidiEvent> {
        self.events.pop()
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn is_connected(&self) -> bool {
        let last = self.last_packet_millis.load(Ordering::Acquire);
        last > 0 && unix_millis().saturating_sub(last) <= 750
    }
}

impl Drop for LiveMidiBridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(receiver) = self.receiver.take() {
            let _ = receiver.join();
        }
    }
}

fn is_loopback(address: SocketAddr) -> bool {
    address.ip().is_loopback()
}

fn make_token(port: u16) -> [u8; 16] {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut state = time ^ (u128::from(std::process::id()) << 64) ^ u128::from(port);
    let mut token = [0u8; 16];
    for byte in &mut token {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
    token
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn decode_packet(packet: &[u8], token: &[u8; 16]) -> Option<Vec<LiveMidiEvent>> {
    if packet.len() < HEADER_BYTES || &packet[..4] != b"RSM1" || packet[4..20] != token[..] {
        return None;
    }
    let count = u16::from_le_bytes([packet[24], packet[25]]) as usize;
    if count > MAX_PACKET_EVENTS || packet.len() != HEADER_BYTES + count * EVENT_BYTES {
        return None;
    }

    let mut events = Vec::with_capacity(count);
    for fields in packet[HEADER_BYTES..].chunks_exact(EVENT_BYTES) {
        let status = fields[0];
        let channel = fields[1];
        let note = fields[2];
        let velocity = fields[3];
        if channel > 15 || note > 127 || velocity > 127 {
            return None;
        }
        match status & 0xf0 {
            0x80 => events.push(LiveMidiEvent::NoteOff { channel, note }),
            0x90 if velocity == 0 => events.push(LiveMidiEvent::NoteOff { channel, note }),
            0x90 => events.push(LiveMidiEvent::NoteOn {
                channel,
                note,
                velocity: f32::from(velocity) / 127.0,
            }),
            _ => return None,
        }
    }
    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn packet(token: [u8; 16], entries: &[[u8; 4]]) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_BYTES];
        bytes[..4].copy_from_slice(b"RSM1");
        bytes[4..20].copy_from_slice(&token);
        bytes[24..26].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        for entry in entries {
            bytes.extend_from_slice(entry);
        }
        bytes
    }

    #[test]
    fn decodes_live_note_on_and_off_packets() {
        let token = [0x5a; 16];
        let bytes = packet(token, &[[0x91, 2, 64, 96], [0x82, 2, 64, 0]]);
        assert_eq!(
            decode_packet(&bytes, &token).expect("valid MIDI packet"),
            vec![
                LiveMidiEvent::NoteOn {
                    channel: 2,
                    note: 64,
                    velocity: 96.0 / 127.0,
                },
                LiveMidiEvent::NoteOff {
                    channel: 2,
                    note: 64,
                },
            ]
        );
    }

    #[test]
    fn rejects_packets_with_the_wrong_token_or_malformed_events() {
        let token = [0x12; 16];
        let bytes = packet(token, &[[0x90, 0, 60, 100]]);
        assert!(decode_packet(&bytes, &[0x13; 16]).is_none());
        assert!(decode_packet(&bytes[..bytes.len() - 1], &token).is_none());
        assert!(decode_packet(&packet(token, &[[0x90, 16, 60, 100]]), &token).is_none());
        assert_eq!(decode_packet(&packet(token, &[]), &token), Some(Vec::new()));
    }

    #[test]
    fn bridge_receives_authenticated_loopback_midi() {
        let bridge = LiveMidiBridge::new().expect("local MIDI receiver");
        let sender = UdpSocket::bind(("127.0.0.1", 0)).expect("local MIDI sender");
        let bytes = packet(bridge.token, &[[0x90, 2, 64, 96]]);
        sender
            .send_to(&bytes, ("127.0.0.1", bridge.port()))
            .expect("send live MIDI packet");

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline && !bridge.is_connected() {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(bridge.is_connected());
        assert_eq!(
            bridge.pop(),
            Some(LiveMidiEvent::NoteOn {
                channel: 2,
                note: 64,
                velocity: 96.0 / 127.0,
            })
        );
    }
}
