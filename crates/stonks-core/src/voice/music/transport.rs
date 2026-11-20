use super::{
    protocol::{MusicPacket, MAX_MUSIC_PACKET_BYTES},
    *,
};

pub(in crate::voice) const MUSIC_TOPIC: &str = "stonks:music:v2";

pub(super) fn encode_packet(packet: &MusicPacket) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(packet).map_err(|error| error.to_string())?;
    if payload.len() > MAX_MUSIC_PACKET_BYTES {
        return Err("The music synchronization packet is too large.".into());
    }
    Ok(payload)
}

pub(super) async fn publish_packet(room: &Room, packet: &MusicPacket) -> Result<(), String> {
    let payload = encode_packet(packet)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        room.local_participant().publish_data(DataPacket {
            payload,
            topic: Some(MUSIC_TOPIC.into()),
            reliable: true,
            ..Default::default()
        }),
    )
    .await
    .map_err(|_| "Music synchronization timed out.")?
    .map_err(|_| "Could not synchronize the music player.".into())
}
