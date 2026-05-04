//! Pairing dialog: show keypair / QR, configure host:port, connect.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct PairingDialogProps {
    pub visible: bool,
    pub public_key: Option<String>,
    pub qr_code_data_url: Option<String>,
    pub gateway_host: String,
    pub gateway_port: u16,
    pub on_host_change: EventHandler<String>,
    pub on_port_change: EventHandler<u16>,
    pub on_connect: EventHandler<()>,
    pub on_generate_key: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn PairingDialog(props: PairingDialogProps) -> Element {
    let mut host = use_signal(|| props.gateway_host.clone());
    let mut port_str = use_signal(|| props.gateway_port.to_string());
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

    let public_key_for_copy = props.public_key.clone();
    let handle_copy = move |_| {
        if let Some(key) = &public_key_for_copy {
            tracing::info!("Copy public key: {}", key);
            copied.set(true);
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |_| props.on_cancel.call(()),

            div {
                class: "modal",
                style: "max-width: 540px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🔗 Pair with gateway" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_cancel.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    // Public key card
                    div { class: "pair-card",
                        div { class: "pair-card-title", "🔑 Your public key" }
                        if let Some(key) = props.public_key.as_ref() {
                            pre { class: "key-pre", "{key}" }
                            div {
                                style: "display: flex; justify-content: flex-end; margin-top: 8px;",
                                button {
                                    class: if *copied.read() { "btn btn-primary btn-sm" } else { "btn btn-subtle btn-sm" },
                                    onclick: handle_copy,
                                    if *copied.read() { "✓ Copied!" } else { "📋 Copy" }
                                }
                            }
                        } else {
                            div {
                                style: "display: flex; justify-content: space-between; align-items: center; gap: 12px;",
                                span { class: "field-help",
                                    "No keypair generated yet."
                                }
                                button {
                                    class: "btn btn-primary btn-sm",
                                    onclick: move |_| props.on_generate_key.call(()),
                                    "+ Generate keypair"
                                }
                            }
                        }
                    }

                    // QR code
                    if let Some(qr_url) = props.qr_code_data_url.as_ref() {
                        div { class: "pair-card",
                            div { class: "pair-card-title", "📱 Scan to pair" }
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
                    div { class: "pair-card",
                        div { class: "pair-card-title", "🖧 Gateway" }
                        div { class: "field-row",
                            div { class: "field", style: "flex: 2;",
                                span { class: "field-label", "Host" }
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
                            div { class: "field", style: "flex: 1;",
                                span { class: "field-label", "Port" }
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

                div { class: "modal-foot",
                    button {
                        class: "btn btn-ghost",
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| props.on_connect.call(()),
                        "🔌 Connect"
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
