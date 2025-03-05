use nostr::prelude::*;

fn main() -> Result<()> {
    // Generate new random keys
    let my_keys = Keys::generate();

    // Or use your already existing (from hex or bech32)
    let my_keys = Keys::parse("hex-or-bech32-secret-key")?;

    // Show bech32 public key
    let bech32_pubkey: String = my_keys.public_key().to_bech32()?;
    println!("Bech32 PubKey: {}", bech32_pubkey);

    let metadata = Metadata::new()
        .name("username")
        .display_name("My Username")
        .about("Description")
        .picture(Url::parse("https://example.com/avatar.png")?)
        .banner(Url::parse("https://example.com/banner.png")?)
        .nip05("username@example.com")
        .lud16("yuki@getalby.com")
        .custom_field("custom_field", "my value");

    let event: Event = EventBuilder::metadata(&metadata).to_event(&my_keys)?;

    // New text note
    let event: Event = EventBuilder::text_note("Hello from rust-nostr", []).to_event(&my_keys)?;

    // New POW text note
    let event: Event = EventBuilder::text_note("My first POW text note from rust-nostr", []).to_pow_event(&my_keys, 20)?;

    // Convert client nessage to JSON
    let json = ClientMessage::event(event).as_json();
    println!("{json}");

    Ok(())
}
