//! Pairing dialog: show keypair / QR, configure host:port, connect.

use dioxus::prelude::*;
use dioxus_bulma::components::{Title, TitleSize};
use dioxus_bulma::prelude::{
    BulmaBox, BulmaColor, BulmaSize, Button, Buttons, Control, Field, FieldLabel,
};
use rustyclaw_view::{tokio, tracing};
use rustyclaw_view::PairingDialogData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct PairingDialogProps {
    pub visible: bool,
    pub data: PairingDialogData,
    pub qr_code_data_url: Option<String>,
    pub on_host_change: EventHandler<String>,
    pub on_port_change: EventHandler<u16>,
    pub on_connect: EventHandler<()>,
    pub on_generate_key: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn PairingDialog(props: PairingDialogProps) -> Element {
    let mut host = use_signal(|| props.data.host.clone());
    let mut port_str = use_signal(|| props.data.port.clone());
    let mut copied = use_signal(|| false);

    use_effect(move || {
        if *copied.read() {
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                copied.set(false);
            });
        }
    });

    if !props.visible {
        return rsx! {};
    }

    let public_key_for_copy = if props.data.public_key.is_empty() {
        None
    } else {
        Some(props.data.public_key.clone())
    };
    let public_key_for_render = public_key_for_copy.clone();
    let handle_copy = move |_| {
        if let Some(key) = &public_key_for_copy {
            tracing::info!("Copy public key: {}", key);
            copied.set(true);
        }
    };

    rsx! {
        RcModal {
            active: true,
            title: "🔗 Pair with gateway",
            width: 540,
            onclose: move |_| props.on_cancel.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_connect.call(()),
                        "🔌 Connect"
                    }
                }
            },

            // Public key card
            BulmaBox { class: "pair-card",
                Title { size: TitleSize::Is6, class: "pair-card-title", "🔑 Your public key" }
                if let Some(key) = public_key_for_render.as_ref() {
                    pre { class: "key-pre", "{key}" }
                    Buttons { alignment: dioxus_bulma::prelude::ButtonsAlignment::Right,
                        Button {
                            color: if *copied.read() { BulmaColor::Primary } else { BulmaColor::Light },
                            size: BulmaSize::Small,
                            onclick: handle_copy,
                            if *copied.read() { "✓ Copied!" } else { "📋 Copy" }
                        }
                    }
                } else {
                    div { class: "pair-card-empty",
                        span { class: "pair-card-hint", "No keypair generated yet." }
                        Button {
                            color: BulmaColor::Primary,
                            size: BulmaSize::Small,
                            onclick: move |_| props.on_generate_key.call(()),
                            "+ Generate keypair"
                        }
                    }
                }
            }

            // QR code
            if let Some(qr_url) = props.qr_code_data_url.as_ref() {
                BulmaBox { class: "pair-card",
                    Title { size: TitleSize::Is6, class: "pair-card-title", "📱 Scan to pair" }
                    div { class: "qr-container",
                        img {
                            src: "{qr_url}",
                            alt: "Pairing QR code",
                        }
                        span { class: "qr-label",
                            "Scan with your gateway's pairing client."
                        }
                    }
                }
            }

            // Gateway address
            BulmaBox { class: "pair-card",
                Title { size: TitleSize::Is6, class: "pair-card-title", "🖧 Gateway" }
                div { class: "field-row",
                    Field { class: "field-host",
                        FieldLabel { "Host" }
                        Control {
                            input {
                                class: "input",
                                r#type: "text",
                                placeholder: "127.0.0.1",
                                value: "{host}",
                                oninput: move |evt| {
                                    let value = evt.value();
                                    host.set(value.clone());
                                    props.on_host_change.call(value);
                                }
                            }
                        }
                    }
                    Field { class: "field-port",
                        FieldLabel { "Port" }
                        Control {
                            input {
                                class: "input",
                                r#type: "number",
                                placeholder: "2222",
                                value: "{port_str}",
                                oninput: move |evt| {
                                    let value = evt.value();
                                    port_str.set(value.clone());
                                    if let Ok(port) = value.parse::<u16>() {
                                        props.on_port_change.call(port);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Generate a QR code as a base64 data URL.
pub fn generate_qr_code(data: &str) -> Option<String> {
    use image::Luma;
    use qrcode::QrCode;

    let code = QrCode::new(data.as_bytes()).ok()?;
    let image = code.render::<Luma<u8>>().build();

    let mut png_data = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
    use image::ImageEncoder;
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::L8,
        )
        .ok()?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_data);
    Some(format!("data:image/png;base64,{}", b64))
}
