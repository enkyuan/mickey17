use super::{MAX_RESPONSE, Request, Response};
use anyhow::{Context, Result, ensure};
use reqwest::{
    blocking::Client,
    header::{AUTHORIZATION, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    TypeSafeHttp,
    ControlledFixture,
}
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub retryable: bool,
    pub retry_after: Option<Duration>,
}
pub enum Reply {
    Response(Response),
    Failure(Failure),
}
/// The caller reserves a full attempt before invoking a provider. Controlled
/// implementations are test evidence, never proof of live Jev qualification.
pub trait Provider {
    fn origin(&self) -> Origin;
    fn ask(&self, request: &Request, timeout: Duration) -> Result<Reply>;
}
pub struct TypeSafe {
    client: Client,
    authorization: HeaderValue,
    endpoint: Url,
    origin: Origin,
}
impl TypeSafe {
    pub fn new(api_key: &str) -> Result<Self> {
        Self::at(
            api_key,
            Url::parse("https://api.typesafe.ai/v1/systemone")?,
            Origin::TypeSafeHttp,
        )
    }
    fn at(api_key: &str, endpoint: Url, origin: Origin) -> Result<Self> {
        ensure!(!api_key.trim().is_empty(), "TypeSafe API key is empty");
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .context("invalid TypeSafe API key header")?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .redirect(Policy::none())
            .https_only(endpoint.scheme() == "https")
            .connect_timeout(Duration::from_secs(5))
            .user_agent("mochi-semantic-questions-v1")
            .build()?;
        Ok(Self {
            client,
            authorization,
            endpoint,
            origin,
        })
    }
}
impl Provider for TypeSafe {
    fn origin(&self) -> Origin {
        self.origin
    }
    fn ask(&self, request: &Request, timeout: Duration) -> Result<Reply> {
        ensure!(
            !timeout.is_zero(),
            "interpretation request deadline exceeded"
        );
        let response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .timeout(timeout.min(Duration::from_secs(30)))
            .json(request)
            .send()
            .map_err(|_| {
                anyhow::anyhow!("TypeSafe request failed or timed out; billing outcome unknown")
            })?;
        let status = response.status().as_u16();
        if status != 200 {
            let mut retryable = matches!(status, 429 | 500 | 502 | 503 | 504 | 529);
            let retry_after = if let Some(value) = response.headers().get("retry-after") {
                let seconds = value
                    .to_str()
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|seconds| *seconds <= 30);
                if seconds.is_none() {
                    retryable = false;
                }
                seconds.map(Duration::from_secs)
            } else {
                None
            };
            // Do not echo a provider error body: it can contain page text or
            // reflected credentials. Redirects are never followed.
            return Ok(Reply::Failure(Failure {
                status,
                retryable,
                retry_after,
            }));
        }
        ensure!(
            response.content_length().unwrap_or(0) <= MAX_RESPONSE,
            "TypeSafe response byte limit exceeded"
        );
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                anyhow::anyhow!("TypeSafe response read failed; billing outcome unknown")
            })?;
        ensure!(
            bytes.len() as u64 <= MAX_RESPONSE,
            "TypeSafe response byte limit exceeded"
        );
        let response = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid TypeSafe response JSON/schema"))?;
        Ok(Reply::Response(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpret::{Answer, MODEL, Question, TruthCriteria, Usage};
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };
    fn request() -> Request {
        Request {
            model: MODEL.into(),
            state: json!({"text":"FAKE FOR TESTS ONLY"}),
            questions: BTreeMap::from([(
                "applicable".into(),
                Question::Noul {
                    instructions: json!("Is this a fixture?"),
                    criteria: TruthCriteria {
                        yes: "fixture".into(),
                        no: "not fixture".into(),
                    },
                },
            )]),
        }
    }
    #[test]
    fn http_contract_auth_validation_and_redirect_refusal() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Url::parse(&format!(
            "http://{}/v1/systemone",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let header_end = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let lower = headers.to_ascii_lowercase();
            let length = lower
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            while bytes.len() < header_end + length {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            assert!(lower.starts_with("post /v1/systemone http/1.1"));
            assert!(lower.contains("authorization: bearer test-only-token"));
            assert!(!lower.contains("danger-zone") && !lower.contains("idempotency"));
            let parsed: Request = serde_json::from_slice(&bytes[header_end..]).unwrap();
            assert_eq!(parsed.model, MODEL);
            assert_eq!(parsed.questions.len(), 1);
            tx.send(()).unwrap();
            let response = Response {
                model: MODEL.into(),
                answers: BTreeMap::from([("applicable".into(), Answer::Noul { noul: 0.9 })]),
                usage: Usage {
                    input_tokens: 123,
                    output_tokens: 4,
                },
            };
            let body = serde_json::to_vec(&response).unwrap();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            stream.write_all(&body).unwrap();
        });
        let provider =
            TypeSafe::at("test-only-token", endpoint, Origin::ControlledFixture).unwrap();
        let Reply::Response(response) = provider.ask(&request(), Duration::from_secs(2)).unwrap()
        else {
            panic!("expected response");
        };
        response.validate(&request()).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_addr = target.local_addr().unwrap();
        let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Url::parse(&format!(
            "http://{}/v1/systemone",
            redirect.local_addr().unwrap()
        ))
        .unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = redirect.accept().unwrap();
            let mut buffer = [0; 8192];
            let _ = stream.read(&mut buffer);
            write!(stream,"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_addr}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let Reply::Failure(failure) =
            TypeSafe::at("test-only-token", endpoint, Origin::ControlledFixture)
                .unwrap()
                .ask(&request(), Duration::from_secs(2))
                .unwrap()
        else {
            panic!("expected refusal");
        };
        assert_eq!(failure.status, 307);
        assert!(!failure.retryable);
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        server.join().unwrap();
    }

    #[test]
    fn http_limits_malformed_bodies_timeouts_and_retry_after_are_bounded() {
        for case in 0..4 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = Url::parse(&format!(
                "http://{}/v1/systemone",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let response = match case {
                    0 => "HTTP/1.1 200 OK\r\nContent-Length: 21\r\nConnection: close\r\n\r\nREFLECTED_SECRET_TOKEN".to_string(),
                    1 => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_RESPONSE + 1),
                    2 => { thread::sleep(Duration::from_millis(100)); return; }
                    _ => "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 999999\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(response.as_bytes());
            });
            let provider =
                TypeSafe::at("FAKE-TEST-KEY", endpoint, Origin::ControlledFixture).unwrap();
            let timeout = if case == 2 {
                Duration::from_millis(30)
            } else {
                Duration::from_secs(2)
            };
            match provider.ask(&request(), timeout) {
                Err(error) => {
                    assert!(case < 3);
                    let text = format!("{error:#}");
                    assert!(!text.contains("REFLECTED_SECRET") && !text.contains("FAKE-TEST-KEY"));
                    assert!(text.contains(match case {
                        0 => "JSON/schema",
                        1 => "byte limit",
                        _ => "timed out",
                    }));
                }
                Ok(Reply::Failure(failure)) => {
                    assert_eq!(case, 3);
                    assert_eq!(failure.status, 429);
                    assert!(!failure.retryable && failure.retry_after.is_none());
                }
                _ => panic!("unexpected successful response"),
            }
            server.join().unwrap();
        }
    }
}
