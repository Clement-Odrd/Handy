//! Mistral Voxtral batch transcription: one finished recording in, its text
//! out (`POST /v1/audio/transcriptions`, multipart).
//!
//! The audio is encoded as WAV in memory and sent over HTTPS only; nothing is
//! written to disk. The API key goes in the `Authorization` header and nowhere
//! else: not in errors, not in logs.
//!
//! Custom words go to the model as `context_bias`. Biasing is experimental for
//! some languages (French among them), so the transcription manager still runs
//! its fuzzy custom-word correction on the text that comes back.

use crate::notices::{Notice, NoticeCode, NoticeSource};
use futures_util::future::{select, Either};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use std::collections::HashSet;
use std::error::Error as _;
use std::fmt;
use std::io::Cursor;
use std::pin::pin;
use std::time::Duration;

/// The batch transcription model (docs.mistral.ai, "Transcribe files").
pub const VOXTRAL_MODEL: &str = "voxtral-mini-transcribe-3";

const API_BASE_URL: &str = "https://api.mistral.ai";
const TRANSCRIPTIONS_PATH: &str = "/v1/audio/transcriptions";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole exchange, upload included. A 16 kHz mono WAV is about 2 MB a
/// minute, so a long dictation on a slow uplink still fits.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// How often a request in flight checks whether the user cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// The bias list counts against the model's context along with the audio,
/// and the API rejects the request (400) rather than trimming it.
pub const MAX_CONTEXT_BIAS_ENTRIES: usize = 100;
/// An error body is shown as a notice detail; keep it to a few lines.
const MAX_ERROR_DETAIL_CHARS: usize = 300;

const SAMPLE_RATE: u32 = 16_000;
const USER_AGENT: &str = "Handy/1.0 (+https://github.com/cjpais/Handy)";

/// Why a Voxtral transcription produced no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxtralError {
    /// No API key is saved.
    MissingKey,
    /// The API refused the key (401 or 403).
    KeyRejected,
    /// Rate limit or quota reached (429).
    RateLimited,
    /// The API couldn't be reached, or didn't answer in time.
    Network(String),
    /// Any other non-success status, with the API's own message.
    Http { status: u16, message: String },
    /// A success status whose body isn't a transcription.
    InvalidResponse(String),
    /// Something on this computer failed before or around the request
    /// (encoding the audio, reading the credential store).
    Local(String),
    /// The user cancelled while the request was in flight.
    Cancelled,
}

impl fmt::Display for VoxtralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => f.write_str("No Mistral API key is saved"),
            Self::KeyRejected => f.write_str("Mistral rejected the API key"),
            Self::RateLimited => f.write_str("Mistral rate limit or quota reached"),
            Self::Network(e) => write!(f, "Couldn't reach Mistral: {e}"),
            Self::Http { status, message } => {
                write!(f, "Mistral returned HTTP {status}: {message}")
            }
            Self::InvalidResponse(e) => write!(f, "Unexpected response from Mistral: {e}"),
            Self::Local(e) => write!(f, "Voxtral transcription failed: {e}"),
            Self::Cancelled => f.write_str("Voxtral transcription cancelled"),
        }
    }
}

impl std::error::Error for VoxtralError {}

impl VoxtralError {
    /// The notice that reports this failure to the user. A cancel isn't a
    /// failure; callers don't raise one for it.
    pub fn notice(&self, source: NoticeSource) -> Notice {
        let (code, detail) = match self {
            Self::MissingKey => (NoticeCode::VoxtralKeyMissing, None),
            Self::KeyRejected => (NoticeCode::VoxtralKeyRejected, None),
            Self::RateLimited => (NoticeCode::VoxtralRateLimited, None),
            Self::Network(e) => (NoticeCode::VoxtralUnreachable, Some(e.clone())),
            Self::Http { status, message } => (
                NoticeCode::VoxtralFailed,
                Some(format!("HTTP {status}: {message}")),
            ),
            Self::InvalidResponse(_) | Self::Local(_) | Self::Cancelled => {
                (NoticeCode::VoxtralFailed, Some(self.to_string()))
            }
        };
        Notice::new(code, source).with_detail(detail)
    }
}

/// What Voxtral returned for one recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoxtralTranscript {
    pub text: String,
    /// The language the API reports. It only does when asked to detect it,
    /// so this is usually `None`.
    pub language: Option<String>,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

/// One recording, ready to send.
pub struct VoxtralRequest {
    wav: Vec<u8>,
    language: Option<String>,
    context_bias: Vec<String>,
}

impl VoxtralRequest {
    /// Encodes `samples` (16 kHz mono) and cleans up the bias list.
    /// `language` is an ISO 639-1 code (see [`api_language`]), or `None` to
    /// let the model detect it.
    ///
    /// # Errors
    /// [`VoxtralError::Local`] if the audio can't be encoded.
    pub fn new(
        samples: &[f32],
        language: Option<String>,
        custom_words: &[String],
    ) -> Result<Self, VoxtralError> {
        let wav = encode_wav(samples)
            .map_err(|e| VoxtralError::Local(format!("couldn't encode the audio: {e}")))?;
        Ok(Self {
            wav,
            language,
            context_bias: context_bias(custom_words),
        })
    }

    fn into_form(self) -> Result<Form, VoxtralError> {
        let file = Part::bytes(self.wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| VoxtralError::Local(e.to_string()))?;
        let mut form = Form::new().text("model", VOXTRAL_MODEL).part("file", file);
        if let Some(language) = self.language {
            form = form.text("language", language);
        }
        // The API takes the list as the same field repeated once per entry.
        for term in self.context_bias {
            form = form.text("context_bias", term);
        }
        Ok(form)
    }
}

/// `samples` as a 16-bit PCM WAV file, in memory. Same format as the
/// recordings Handy saves.
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, hound::Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer = hound::WavWriter::new(&mut buffer, spec)?;
        for sample in samples {
            // `as` saturates, so out-of-range samples clip instead of wrapping.
            writer.write_sample((sample * i16::MAX as f32) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(buffer.into_inner())
}

/// The custom words as a `context_bias` list: trimmed, blanks dropped,
/// duplicates dropped regardless of case (the first spelling wins), and at
/// most [`MAX_CONTEXT_BIAS_ENTRIES`] entries.
pub fn context_bias(custom_words: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    custom_words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .filter(|word| seen.insert(word.to_lowercase()))
        .take(MAX_CONTEXT_BIAS_ENTRIES)
        .map(str::to_string)
        .collect()
}

/// The API's `language` for Handy's language setting: the two-letter
/// ISO 639-1 code of a picked language (`pt-BR` -> `pt`), or `None` for
/// "auto" and for codes with no two-letter form (`yue`), which the model then
/// detects.
pub fn api_language(selected_language: &str) -> Option<String> {
    let primary = selected_language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    (primary.len() == 2 && primary.chars().all(|c| c.is_ascii_lowercase())).then_some(primary)
}

/// Transcribes `request` with Mistral's API, blocking the calling thread
/// until the text is back, the request fails, or `cancelled` returns true
/// (checked every 100 ms).
///
/// # Errors
/// See [`VoxtralError`]; a cancel is [`VoxtralError::Cancelled`].
pub fn transcribe_blocking(
    api_key: &str,
    request: VoxtralRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<VoxtralTranscript, VoxtralError> {
    run_blocking(API_BASE_URL, true, api_key, request, cancelled)
}

/// [`transcribe_blocking`] against `base_url`. Only tests turn `https_only`
/// off, to talk to a local plain-HTTP server.
fn run_blocking(
    base_url: &str,
    https_only: bool,
    api_key: &str,
    request: VoxtralRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<VoxtralTranscript, VoxtralError> {
    // The dictation path calls this from a task on Tauri's async runtime,
    // where blocking on a future would panic. A thread of its own, with a
    // small runtime of its own, works from any caller and never competes
    // with the app's runtime for workers.
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| VoxtralError::Local(format!("couldn't start the runtime: {e}")))?;
                runtime.block_on(async move {
                    let send = pin!(send(base_url, https_only, api_key, request));
                    let watch = pin!(wait_for_cancel(cancelled));
                    match select(send, watch).await {
                        Either::Left((result, _)) => result,
                        Either::Right(((), _)) => Err(VoxtralError::Cancelled),
                    }
                })
            })
            .join()
            .unwrap_or_else(|_| Err(VoxtralError::Local("the request thread panicked".into())))
    })
}

async fn wait_for_cancel(cancelled: &(dyn Fn() -> bool + Sync)) {
    while !cancelled() {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

async fn send(
    base_url: &str,
    https_only: bool,
    api_key: &str,
    request: VoxtralRequest,
) -> Result<VoxtralTranscript, VoxtralError> {
    // A client per request: each request runs on its own short-lived
    // runtime, and a pooled connection can't outlive the runtime it was
    // opened on.
    let client = reqwest::Client::builder()
        .https_only(https_only)
        // A redirect would carry the key to wherever it points.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| VoxtralError::Local(describe(&e)))?;
    let response = client
        .post(format!("{base_url}{TRANSCRIPTIONS_PATH}"))
        .bearer_auth(api_key)
        .multipart(request.into_form()?)
        .send()
        .await
        .map_err(|e| VoxtralError::Network(describe(&e)))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| VoxtralError::Network(describe(&e)))?;
    if !status.is_success() {
        return Err(error_for_status(status.as_u16(), &body));
    }
    parse_response(&body)
}

/// The text of a success body.
fn parse_response(body: &[u8]) -> Result<VoxtralTranscript, VoxtralError> {
    // serde's message names the problem (a missing field, a bad type) without
    // quoting the body, which holds what the user said.
    let response: TranscriptionResponse =
        serde_json::from_slice(body).map_err(|e| VoxtralError::InvalidResponse(e.to_string()))?;
    Ok(VoxtralTranscript {
        text: response.text.trim().to_string(),
        language: response.language.filter(|language| !language.is_empty()),
    })
}

/// The error for a non-success status. 401/403 and 429 get their own notice;
/// anything else carries the API's message.
fn error_for_status(status: u16, body: &[u8]) -> VoxtralError {
    match status {
        401 | 403 => VoxtralError::KeyRejected,
        429 => VoxtralError::RateLimited,
        _ => VoxtralError::Http {
            status,
            message: error_message(body),
        },
    }
}

/// The API's own message from an error body (`message`, `detail`, or
/// `error.message`), else the body as text, cut to a few lines.
fn error_message(body: &[u8]) -> String {
    let from_json = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            [
                json.get("message"),
                json.get("detail"),
                json.get("error").and_then(|e| e.get("message")),
            ]
            .into_iter()
            .flatten()
            .find(|value| !value.is_null())
            .map(|value| match value.as_str() {
                Some(text) => text.to_string(),
                None => value.to_string(),
            })
        });
    let message = from_json.unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    if message.chars().count() > MAX_ERROR_DETAIL_CHARS {
        let cut: String = message.chars().take(MAX_ERROR_DETAIL_CHARS).collect();
        format!("{cut}…")
    } else {
        message
    }
}

/// A request error with its causes ("error sending request: connection
/// refused"). reqwest's errors carry the URL, never the headers.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "the request timed out".to_string();
    }
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const NEVER: &(dyn Fn() -> bool + Sync) = &|| false;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| word.to_string()).collect()
    }

    fn request() -> VoxtralRequest {
        VoxtralRequest::new(
            &[0.0, 0.5, -0.5, 1.0],
            Some("fr".to_string()),
            &words(&["Odoo", " fibre optique ", "odoo", ""]),
        )
        .unwrap()
    }

    /// Reads one HTTP request (headers and body) from `stream`.
    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                return data;
            }
            data.extend_from_slice(&chunk[..n]);
            let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok());
            let complete = match length {
                Some(length) => data.len() >= end + 4 + length,
                None => data.ends_with(b"\r\n0\r\n\r\n"),
            };
            if complete {
                return data;
            }
        }
    }

    /// A one-shot local server: answers the first request with `status` and
    /// `body`, and hands back the raw request it received.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let raw = read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&raw).into_owned()
        });
        (base, handle)
    }

    #[test]
    fn production_endpoint_is_https() {
        assert!(API_BASE_URL.starts_with("https://"));
    }

    #[test]
    fn wav_is_16_bit_mono_16_khz_in_memory() {
        let wav = encode_wav(&[0.0, 1.0, -1.0, 2.0]).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        // Out-of-range input clips instead of wrapping around.
        assert_eq!(samples, vec![0, i16::MAX, -i16::MAX, i16::MAX]);
    }

    #[test]
    fn context_bias_is_trimmed_deduplicated_and_without_blanks() {
        let bias = context_bias(&words(&[
            " Odoo ",
            "",
            "   ",
            "odoo",
            "Fibre optique",
            "ODOO",
            "PTO",
        ]));
        assert_eq!(bias, words(&["Odoo", "Fibre optique", "PTO"]));
    }

    #[test]
    fn context_bias_is_capped() {
        let many: Vec<String> = (0..250).map(|i| format!("term{i}")).collect();
        let bias = context_bias(&many);
        assert_eq!(bias.len(), MAX_CONTEXT_BIAS_ENTRIES);
        assert_eq!(bias[0], "term0");
    }

    #[test]
    fn api_language_keeps_only_two_letter_codes() {
        assert_eq!(api_language("auto"), None);
        assert_eq!(api_language(""), None);
        assert_eq!(api_language("fr"), Some("fr".to_string()));
        assert_eq!(api_language("pt-BR"), Some("pt".to_string()));
        assert_eq!(api_language("zh_Hans"), Some("zh".to_string()));
        assert_eq!(api_language("yue"), None);
    }

    #[test]
    fn parses_the_text_and_optional_language() {
        let body = br#"{"model":"voxtral-mini-transcribe-3","text":" Bonjour. ","language":null,"segments":[],"usage":{"prompt_audio_seconds":3}}"#;
        assert_eq!(
            parse_response(body),
            Ok(VoxtralTranscript {
                text: "Bonjour.".to_string(),
                language: None
            })
        );
        let detected = br#"{"text":"Hello","language":"en"}"#;
        assert_eq!(
            parse_response(detected).unwrap().language.as_deref(),
            Some("en")
        );
    }

    #[test]
    fn a_body_without_text_is_an_invalid_response() {
        assert!(matches!(
            parse_response(br#"{"object":"list"}"#),
            Err(VoxtralError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_response(b"not json"),
            Err(VoxtralError::InvalidResponse(_))
        ));
    }

    #[test]
    fn statuses_map_to_their_notices() {
        let code = |status: u16| {
            error_for_status(status, b"{}")
                .notice(NoticeSource::Dictation)
                .code
        };
        assert_eq!(code(401), NoticeCode::VoxtralKeyRejected);
        assert_eq!(code(403), NoticeCode::VoxtralKeyRejected);
        assert_eq!(code(429), NoticeCode::VoxtralRateLimited);
        assert_eq!(code(400), NoticeCode::VoxtralFailed);
        assert_eq!(code(500), NoticeCode::VoxtralFailed);
        assert_eq!(
            VoxtralError::MissingKey
                .notice(NoticeSource::Dictation)
                .code,
            NoticeCode::VoxtralKeyMissing
        );
        assert_eq!(
            VoxtralError::Network("timed out".into())
                .notice(NoticeSource::File)
                .code,
            NoticeCode::VoxtralUnreachable
        );
    }

    #[test]
    fn other_errors_carry_the_api_message() {
        let error = error_for_status(
            400,
            br#"{"object":"error","message":"Prompt too long","type":"invalid_request_prompt_too_long"}"#,
        );
        assert_eq!(
            error,
            VoxtralError::Http {
                status: 400,
                message: "Prompt too long".to_string()
            }
        );
        let notice = error.notice(NoticeSource::Dictation);
        assert_eq!(notice.detail.as_deref(), Some("HTTP 400: Prompt too long"));

        let long = "x".repeat(1000);
        match error_for_status(502, long.as_bytes()) {
            VoxtralError::Http { message, .. } => {
                assert_eq!(message.chars().count(), MAX_ERROR_DETAIL_CHARS + 1)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn sends_the_documented_multipart_request() {
        let (base, server) = serve_once("200 OK", r#"{"text":"Bonjour Odoo"}"#);
        let transcript = run_blocking(&base, false, "test-key", request(), NEVER).unwrap();
        assert_eq!(transcript.text, "Bonjour Odoo");

        let raw = server.join().unwrap();
        let lower = raw.to_ascii_lowercase();
        assert!(raw.starts_with("POST /v1/audio/transcriptions HTTP/1.1"));
        assert!(lower.contains("authorization: bearer test-key"));
        assert!(lower.contains("content-type: multipart/form-data; boundary="));
        assert!(raw.contains("name=\"model\"\r\n\r\nvoxtral-mini-transcribe-3\r\n"));
        assert!(raw.contains("name=\"language\"\r\n\r\nfr\r\n"));
        assert!(raw.contains("name=\"file\"; filename=\"audio.wav\""));
        assert!(lower.contains("content-type: audio/wav"));
        assert!(raw.contains("RIFF"));
        assert_eq!(raw.matches("name=\"context_bias\"").count(), 2);
        assert!(raw.contains("name=\"context_bias\"\r\n\r\nOdoo\r\n"));
        assert!(raw.contains("name=\"context_bias\"\r\n\r\nfibre optique\r\n"));
    }

    #[test]
    fn auto_language_sends_no_language_field() {
        let (base, server) = serve_once("200 OK", r#"{"text":"Hi"}"#);
        let request = VoxtralRequest::new(&[0.0; 160], None, &[]).unwrap();
        run_blocking(&base, false, "test-key", request, NEVER).unwrap();
        let raw = server.join().unwrap();
        assert!(!raw.contains("name=\"language\""));
        assert!(!raw.contains("name=\"context_bias\""));
    }

    #[test]
    fn a_rejected_key_is_reported_without_the_key() {
        let (base, server) = serve_once("401 Unauthorized", r#"{"message":"Unauthorized"}"#);
        let error = run_blocking(&base, false, "test-key", request(), NEVER).unwrap_err();
        server.join().unwrap();
        assert_eq!(error, VoxtralError::KeyRejected);
        assert!(!error.to_string().contains("test-key"));
    }

    #[test]
    fn quota_is_its_own_error() {
        let (base, server) = serve_once("429 Too Many Requests", r#"{"message":"Rate limit"}"#);
        let error = run_blocking(&base, false, "test-key", request(), NEVER).unwrap_err();
        server.join().unwrap();
        assert_eq!(error, VoxtralError::RateLimited);
    }

    #[test]
    fn an_unreachable_server_is_a_network_error() {
        // Bind then drop, so nothing listens on the port.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = run_blocking(
            &format!("http://127.0.0.1:{port}"),
            false,
            "test-key",
            request(),
            NEVER,
        )
        .unwrap_err();
        match error {
            VoxtralError::Network(detail) => assert!(!detail.contains("test-key")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn plain_http_is_refused_in_production_mode() {
        let error =
            run_blocking("http://127.0.0.1:9", true, "test-key", request(), NEVER).unwrap_err();
        assert!(matches!(error, VoxtralError::Network(_)));
    }

    #[test]
    fn a_cancel_stops_a_request_in_flight() {
        // A server that accepts and never answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let _server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(5));
            drop(stream);
        });
        let cancelled = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let error = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(200));
                cancelled.store(true, Ordering::SeqCst);
            });
            run_blocking(&base, false, "test-key", request(), &|| {
                cancelled.load(Ordering::SeqCst)
            })
            .unwrap_err()
        });
        assert_eq!(error, VoxtralError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
