//! Protocol tests with an in-process client, synthetic pixels and recorded input.
//! No screen capture, power assertions, OS input injection or external hosts.

use std::num::{NonZeroU16, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use ironrdp_connector::sspi::{self, credssp, generator::GeneratorState};
use ironrdp_connector::{ClientConnector, Config, Credentials as ClientCredentials};
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::Action;
use ironrdp_tokio::{
    connect_begin, connect_finalize, mark_as_upgraded, FramedWrite, NetworkClient, TokioFramed,
};
use tokio::net::TcpStream;
use tokio_rustls::{rustls, TlsAcceptor, TlsConnector};

use crate::{
    BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat,
    RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent,
};

#[derive(Default)]
struct Observations {
    display_attempts: AtomicUsize,
    live_displays: AtomicUsize,
    keys: AtomicUsize,
    fail_next_display: AtomicBool,
}

struct TestDisplay {
    observations: Arc<Observations>,
    size: DesktopSize,
}

#[async_trait::async_trait]
impl RdpServerDisplay for TestDisplay {
    async fn size(&mut self) -> DesktopSize {
        self.size
    }

    fn set_resolution(&mut self, resolution: &str) {
        self.size = match resolution {
            "80x60" => DesktopSize {
                width: 80,
                height: 60,
            },
            "auto" => DesktopSize {
                width: 64,
                height: 48,
            },
            _ => panic!("unexpected test resolution: {resolution}"),
        };
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        self.observations
            .display_attempts
            .fetch_add(1, Ordering::SeqCst);
        if self
            .observations
            .fail_next_display
            .swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("No display found");
        }
        self.observations
            .live_displays
            .fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(TestUpdates {
            observations: Arc::clone(&self.observations),
            sent: false,
        }))
    }
}

struct TestUpdates {
    observations: Arc<Observations>,
    sent: bool,
}

impl Drop for TestUpdates {
    fn drop(&mut self) {
        self.observations
            .live_displays
            .fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for TestUpdates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        if std::mem::replace(&mut self.sent, true) {
            return std::future::pending().await;
        }
        Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: NonZeroU16::new(64).unwrap(),
            height: NonZeroU16::new(48).unwrap(),
            format: PixelFormat::BgrA32,
            data: Bytes::from([10, 20, 30, 255].repeat(64 * 48)),
            stride: NonZeroUsize::new(64 * 4).unwrap(),
        })))
    }
}

struct TestInput(Arc<Observations>);

impl RdpServerInputHandler for TestInput {
    fn keyboard(&mut self, event: KeyboardEvent) {
        if matches!(event, KeyboardEvent::Pressed { code: 0x1e, .. }) {
            self.0.keys.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn mouse(&mut self, _: MouseEvent) {}
}

struct NoExternalNetwork;

impl NetworkClient for NoExternalNetwork {
    async fn send(
        &mut self,
        _: &ironrdp_connector::sspi::generator::NetworkRequest,
    ) -> ironrdp_connector::ConnectorResult<Vec<u8>> {
        panic!("local NTLM test unexpectedly requested an external authentication service");
    }
}

fn client_config(nla: bool, username: &str, password: &str) -> Config {
    Config {
        desktop_size: ironrdp_connector::DesktopSize {
            width: 64,
            height: 48,
        },
        desktop_scale_factor: 100,
        enable_tls: true,
        enable_credssp: nla,
        credentials: ClientCredentials::UsernamePassword {
            username: username.into(),
            password: password.into(),
        },
        // Windows clients may qualify the configured local account with their PC name.
        domain: Some("CLIENT-PC".into()),
        client_build: 19045,
        client_name: "connection-test".into(),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: MajorPlatformType::WINDOWS,
        hardware_id: None,
        request_data: None,
        autologon: true,
        enable_audio_playback: false,
        performance_flags: Default::default(),
        license_cache: None,
        timezone_info: Default::default(),
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
}

#[derive(Debug)]
struct CredsspMessage;

impl ironrdp_pdu::PduHint for CredsspMessage {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        match credssp::TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some((true, length))),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(ironrdp_core::other_err!("test CredSSP message", source: error)),
        }
    }
}

/// The generic connector uses Negotiate, which probes DNS even for local accounts.
/// Drive the same CredSSP exchange with NTLM explicitly to keep this test self-contained.
async fn authenticate_local_account(
    framed: &mut TokioFramed<tokio_rustls::client::TlsStream<TcpStream>>,
    connector: &mut ClientConnector,
    public_key: Vec<u8>,
    username: &str,
    password: &str,
) -> Result<()> {
    let credentials = sspi::AuthIdentity {
        username: sspi::Username::new(username, Some("CLIENT-PC"))?,
        password: password.to_owned().into(),
    };
    let mut client = credssp::CredSspClient::new(
        public_key,
        credentials.into(),
        credssp::CredSspMode::WithCredentials,
        credssp::ClientMode::Negotiate(sspi::NegotiateConfig {
            protocol_config: Box::<sspi::ntlm::NtlmConfig>::default(),
            package_list: Some("!kerberos,!pku2u".into()),
            client_computer_name: "localhost".into(),
        }),
        "TERMSRV/localhost".into(),
    )?;
    let mut request = credssp::TsRequest::default();
    loop {
        let state = match client.process(request).start() {
            GeneratorState::Completed(result) => result?,
            GeneratorState::Suspended(_) => anyhow::bail!("NTLM requested external authentication"),
        };
        let (message, finished) = match state {
            credssp::ClientState::ReplyNeeded(message) => (message, false),
            credssp::ClientState::FinalMessage(message) => (message, true),
        };
        let mut bytes = vec![0; usize::from(message.buffer_len())];
        message.encode_ts_request(bytes.as_mut_slice())?;
        framed.write_all(&bytes).await?;
        if finished {
            break;
        }
        request = credssp::TsRequest::from_buffer(&framed.read_by_hint(&CredsspMessage).await?)?;
    }
    // The server selects HYBRID_EX, which sends an explicit result before MCS setup.
    let bytes = framed
        .read_exact(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE)
        .await?;
    anyhow::ensure!(
        matches!(
            credssp::EarlyUserAuthResult::from_buffer(&bytes[..])?,
            credssp::EarlyUserAuthResult::Success
        ),
        "NLA authentication rejected"
    );
    connector.mark_credssp_as_done();
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    AuthenticationRejected,
    DisplayRejected,
    Frame,
}

#[tokio::test]
async fn authenticated_sessions_deliver_frames_and_input_and_survive_failed_reconnects() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = certified.cert.der().clone();
    let public_key = certified.key_pair.public_key_raw().to_vec();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server_tls = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()).into(),
        )
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).unwrap();
    let client_tls = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );

    let observations = Arc::new(Observations::default());
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_hybrid(TlsAcceptor::from(Arc::new(server_tls)), public_key.clone())
        .with_input_handler(TestInput(Arc::clone(&observations)))
        .with_display_handler(TestDisplay {
            observations: Arc::clone(&observations),
            size: DesktopSize {
                width: 64,
                height: 48,
            },
        })
        .build();
    let gfx_state = Arc::new(std::sync::Mutex::new(crate::gfx::GfxState::new(
        64, 48, false,
    )));
    server.set_gfx_state(Arc::clone(&gfx_state));
    server.set_credentials(Some(Credentials {
        username: "viewer".into(),
        password: "test-password".into(),
        domain: None,
    }));
    let events = server.event_sender().clone();

    let clients = async {
        let (tx, rx) = tokio::sync::oneshot::channel();
        events.send(ServerEvent::GetLocalAddr(tx)).unwrap();
        let address = rx.await.unwrap().unwrap();
        // A client closing halfway through the TPKT header must not prevent the next
        // real RDP connection from reaching authentication and display initialization.
        let mut probe = TcpStream::connect(address).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut probe, &[0x03, 0x00])
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut probe)
            .await
            .unwrap();
        drop(probe);
        let cases = [
            (false, "viewer", "", Expected::AuthenticationRejected),
            (
                false,
                "viewer",
                "incorrect",
                Expected::AuthenticationRejected,
            ),
            (
                true,
                "viewer",
                "incorrect",
                Expected::AuthenticationRejected,
            ),
            (true, "viewer", "test-password", Expected::DisplayRejected),
            (true, "viewer", "test-password", Expected::Frame),
            (false, "viewer", "test-password", Expected::Frame),
            (
                true,
                "different-user",
                "test-password",
                Expected::AuthenticationRejected,
            ),
            (true, "viewer", "test-password", Expected::Frame),
        ];
        let mut successful_connections = 0;
        for (nla, username, password, expected) in cases {
            eprintln!("Starting protocol case: NLA={nla}, {expected:?}");
            let expected_size = if matches!(expected, Expected::Frame) {
                successful_connections += 1;
                let (mode, size) = match successful_connections {
                    1 => (None, (64, 48)),
                    2 => (Some("80x60"), (80, 60)),
                    3 => (Some("auto"), (64, 48)),
                    _ => unreachable!(),
                };
                gfx_state.lock().unwrap().resolution = mode.map(str::to_owned);
                size
            } else {
                (64, 48)
            };
            let attempts_before = observations.display_attempts.load(Ordering::SeqCst);
            observations.fail_next_display.store(
                matches!(expected, Expected::DisplayRejected),
                Ordering::SeqCst,
            );
            let stream = TcpStream::connect(address).await.unwrap();
            let mut connector = ClientConnector::new(
                client_config(nla, username, password),
                stream.local_addr().unwrap(),
            );
            let mut framed = TokioFramed::new(stream);
            let upgrade = connect_begin(&mut framed, &mut connector).await.unwrap();
            let stream = framed.into_inner_no_leftover();
            let tls = TlsConnector::from(Arc::clone(&client_tls))
                .connect("localhost".try_into().unwrap(), stream)
                .await
                .unwrap();
            let mut framed = TokioFramed::new(tls);
            let upgraded = mark_as_upgraded(upgrade, &mut connector);
            let result: Result<_> = async {
                if nla {
                    authenticate_local_account(
                        &mut framed,
                        &mut connector,
                        public_key.clone(),
                        username,
                        password,
                    )
                    .await?;
                }
                Ok(connect_finalize(
                    upgraded,
                    connector,
                    &mut framed,
                    &mut NoExternalNetwork,
                    "localhost".to_owned().into(),
                    public_key.clone(),
                    None,
                )
                .await?)
            }
            .await;

            if matches!(expected, Expected::AuthenticationRejected) {
                assert!(
                    result.is_err(),
                    "unauthenticated session accepted (NLA={nla})"
                );
                assert_eq!(
                    observations.display_attempts.load(Ordering::SeqCst),
                    attempts_before
                );
            } else {
                let result = result.unwrap_or_else(|error| {
                    panic!("valid credentials rejected (NLA={nla}): {error:?}")
                });
                assert_eq!(
                    (result.desktop_size.width, result.desktop_size.height),
                    expected_size
                );
                {
                    let gfx = gfx_state.lock().unwrap();
                    assert_eq!((gfx.width, gfx.height), expected_size);
                }
                let received_frame = loop {
                    match framed.read_pdu().await {
                        Ok((Action::FastPath, bytes)) => break !bytes.is_empty(),
                        Ok(_) => continue,
                        Err(_) => break false,
                    }
                };
                assert_eq!(
                    received_frame,
                    matches!(expected, Expected::Frame),
                    "{expected:?}, NLA={nla}"
                );
                assert_eq!(
                    observations.display_attempts.load(Ordering::SeqCst),
                    attempts_before + 1
                );
                if received_frame {
                    let keys_before = observations.keys.load(Ordering::SeqCst);
                    let key = FastPathInput::single(FastPathInputEvent::KeyboardEvent(
                        KeyboardFlags::empty(),
                        0x1e,
                    ));
                    framed
                        .write_all(&ironrdp_core::encode_vec(&key).unwrap())
                        .await
                        .unwrap();
                    while observations.keys.load(Ordering::SeqCst) == keys_before {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            }
            drop(framed);
            while observations.live_displays.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        events
            .send(ServerEvent::Quit("protocol test completed".into()))
            .unwrap();
    };
    tokio::time::timeout(Duration::from_secs(60), async {
        let (server_result, ()) = tokio::join!(server.run(), clients);
        server_result.unwrap();
    })
    .await
    .expect("RDP protocol test stalled");
    assert_eq!(observations.keys.load(Ordering::SeqCst), 3);
    assert_eq!(observations.live_displays.load(Ordering::SeqCst), 0);
}
