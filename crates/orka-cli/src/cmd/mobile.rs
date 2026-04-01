use std::time::{Duration, Instant};

use qrcode::{QrCode, render::unicode};
use serde::Deserialize;

use crate::client::{OrkaClient, Result};

#[derive(Debug, Deserialize)]
struct CreatePairingResponse {
    pairing_id: String,
    pairing_secret: String,
    expires_at: String,
    pairing_uri: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PairingStatus {
    Pending,
    Completed,
    Expired,
}

#[derive(Debug, Deserialize)]
struct PairingStatusResponse {
    status: PairingStatus,
    expires_at: String,
    completed_at: Option<String>,
    device_label: Option<String>,
}

pub async fn run_pair(client: &OrkaClient, timeout_secs: u64) -> Result<()> {
    let response = client
        .post_json(
            "/mobile/v1/pairings",
            &serde_json::json!({
                "server_base_url": client.base_url(),
            }),
        )
        .await?;
    let pairing: CreatePairingResponse = serde_json::from_value(response)?;

    print_pairing_banner(client.base_url(), &pairing)?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let response = client
            .get_json(&format!("/mobile/v1/pairings/{}", pairing.pairing_id))
            .await?;
        let status: PairingStatusResponse = serde_json::from_value(response)?;
        match status.status {
            PairingStatus::Pending => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "pairing timed out after {timeout_secs}s (pairing expires at {})",
                        status.expires_at
                    )
                    .into());
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            PairingStatus::Completed => {
                println!();
                println!(
                    "Paired successfully{}.",
                    status
                        .device_label
                        .as_deref()
                        .map(|label| format!(" with {label}"))
                        .unwrap_or_default()
                );
                if let Some(completed_at) = status.completed_at {
                    println!("Completed at: {completed_at}");
                }
                break;
            }
            PairingStatus::Expired => {
                return Err(format!(
                    "pairing expired before completion (expired at {})",
                    status.expires_at
                )
                .into());
            }
        }
    }

    Ok(())
}

fn print_pairing_banner(server_url: &str, pairing: &CreatePairingResponse) -> Result<()> {
    let qr = QrCode::new(pairing.pairing_uri.as_bytes())?;
    let rendered = qr.render::<unicode::Dense1x2>().quiet_zone(false).build();

    println!();
    println!("Scan this QR code with Mobile Orka:");
    println!();
    println!("{rendered}");
    println!("Server: {server_url}");
    println!("Pairing ID: {}", pairing.pairing_id);
    println!("Expires at: {}", pairing.expires_at);
    println!("Pairing URI:");
    println!("{}", pairing.pairing_uri);
    println!();
    println!(
        "The pairing secret is one-time and short-lived. Do not paste it into chats or logs."
    );
    let _ = &pairing.pairing_secret;
    Ok(())
}
