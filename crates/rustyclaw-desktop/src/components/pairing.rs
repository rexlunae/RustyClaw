//! Pairing dialog with QR code generation.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

/// Props for PairingDialog.
#[derive(Props, Clone, PartialEq)]
pub struct PairingDialogProps {
    /// Whether the dialog is visible
    pub visible: bool,
    /// Client public key (for display)
    pub public_key: Option<String>,
    /// QR code data URL (base64 PNG)
    pub qr_code_data_url: Option<String>,
    /// Gateway host
    pub gateway_host: String,
    /// Gateway port
    pub gateway_port: u16,
    /// Callback when host changes
    pub on_host_change: EventHandler<String>,
    /// Callback when port changes
    pub on_port_change: EventHandler<u16>,
    /// Callback to connect
    pub on_connect: EventHandler<()>,
    /// Callback to generate new keypair
    pub on_generate_key: EventHandler<()>,
    /// Callback to cancel
    pub on_cancel: EventHandler<()>,
}

/// Pairing dialog component.
#[component]
pub fn PairingDialog(props: PairingDialogProps) -> Element {
    let mut host = use_signal(|| props.gateway_host.clone());
    let mut port_str = use_signal(|| props.gateway_port.to_string());
    let mut copied = use_signal(|| false);

    // Reset copied state after 2 seconds
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

    let public_key = props.public_key.clone();
    let handle_copy = move |_| {
        if let Some(key) = &public_key {
            // In a real implementation, use clipboard API
            // For now, just mark as copied
            tracing::info!("Copy public key: {}", key);
            copied.set(true);
        }
    };

    rsx! {
        div { class: "modal is-active",
            div { class: "modal-background",
                onclick: move |_| props.on_cancel.call(()),
            }

            div { class: "modal-card",
                style: "max-width: 550px;",

                header { class: "modal-card-head",
                    p { class: "modal-card-title",
                        span { class: "icon",
                            i { class: "fas fa-link" }
                        }
                        " Pair with Gateway"
                    }
                    button {
                        class: "delete",
                        onclick: move |_| props.on_cancel.call(()),
                    }
                }

                section { class: "modal-card-body",
                    // Public key display
                    div { class: "box",
                        style: "background: #f5f5f5;",

                        p { class: "has-text-weight-semibold",
                            span { class: "icon is-small",
                                i { class: "fas fa-key" }
                            }
                            " Your Public Key"
                        }

                        if let Some(key) = &props.public_key {
                            div { style: "margin-top: 0.5rem;",
                                pre {
                                    style: "background: white; padding: 0.5rem; border-radius: 4px; font-size: 0.75rem; overflow-x: auto;",
                                    "{key}"
                                }

                                div { class: "buttons is-right",
                                    style: "margin-top: 0.5rem;",

                                    Button {
                                        size: BulmaSize::Small,
                                        color: if *copied.read() { BulmaColor::Success } else { BulmaColor::Light },
                                        onclick: handle_copy,

                                        span { class: "icon is-small",
                                            i { class: if *copied.read() { "fas fa-check" } else { "fas fa-copy" } }
                                        }
                                        span { if *copied.read() { "Copied!" } else { "Copy" } }
                                    }
                                }
                            }
                        } else {
                            div { style: "margin-top: 0.5rem;",
                                p { class: "has-text-grey", "No keypair generated" }
                                Button {
                                    size: BulmaSize::Small,
                                    color: BulmaColor::Primary,
                                    onclick: move |_| props.on_generate_key.call(()),

                                    span { class: "icon is-small",
                                        i { class: "fas fa-plus" }
                                    }
                                    span { "Generate Keypair" }
                                }
                            }
                        }
                    }

                    // QR code
                    if let Some(qr_url) = &props.qr_code_data_url {
                        div { class: "has-text-centered",
                            style: "margin: 1rem 0;",

                            p { class: "has-text-grey is-size-7",
                                "─── OR scan QR code ───"
                            }

                            img {
                                src: "{qr_url}",
                                alt: "Pairing QR Code",
                                style: "max-width: 200px; margin: 1rem auto;",
                            }
                        }
                    }

                    // Gateway connection settings
                    div { class: "box",
                        p { class: "has-text-weight-semibold",
                            span { class: "icon is-small",
                                i { class: "fas fa-server" }
                            }
                            " Gateway"
                        }

                        div { class: "columns",
                            style: "margin-top: 0.5rem;",

                            div { class: "column is-8",
                                Field {
                                    FieldLabel { "Host" }
                                    Control { class: "has-icons-left",
                                        input {
                                            class: "input",
                                            r#type: "text",
                                            placeholder: "127.0.0.1",
                                            value: "{host}",
                                            oninput: move |evt| {
                                                let value = evt.value();
                                                host.set(value.clone());
                                                props.on_host_change.call(value);
                                            },
                                        }
                                        span { class: "icon is-left",
                                            i { class: "fas fa-network-wired" }
                                        }
                                    }
                                }
                            }

                            div { class: "column is-4",
                                Field {
                                    FieldLabel { "Port" }
                                    Control {
                                        input {
                                            class: "input",
                                            r#type: "number",
                                            placeholder: "9001",
                                            value: "{port_str}",
                                            oninput: move |evt| {
                                                let value = evt.value();
                                                port_str.set(value.clone());
                                                if let Ok(port) = value.parse::<u16>() {
                                                    props.on_port_change.call(port);
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                footer { class: "modal-card-foot",
                    style: "justify-content: flex-end;",

                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }

                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_connect.call(()),

                        span { class: "icon",
                            i { class: "fas fa-plug" }
                        }
                        span { "Connect" }
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
