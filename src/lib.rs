#[macro_use]
extern crate log;
extern crate reqwest;
extern crate serde;
extern crate serde_json;
extern crate url;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;
#[cfg(test)]
extern crate mockito;
#[cfg(test)]
extern crate env_logger;

extern crate gateway;

// use std::convert::{ TryInto };
use std::fmt;

use gateway::{ parse_url, Message, Service, ServiceError };
use gateway::contracts::{ v1 };


#[derive(Debug, Default)]
/// Captures reqwest metadata for diagnosing issues with calls
pub struct ErrorContext {
    response: Option<reqwest::Response>,
    error: Option<reqwest::Error>,
}

pub type GatewayError = gateway::Error<serde_json::error::Error, serde_json::error::Error>;

pub type ServiceResult<TResponse> = Result<TResponse, ServiceError<ErrorContext, GatewayError>>;

/// Service implementation using Reqwest for proxying to the backing api(s)
pub struct ReqwestService {
    url: url::Url,
}

impl fmt::Debug for ReqwestService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReqwestService {{ url: {} }}", self.url)
    }
}

impl ReqwestService {
    pub fn with_url(url_str: &str) -> ServiceResult<Self> {
        parse_url(url_str)
            .map_err(|error| {
                ServiceError {
                    context: Default::default(),
                    error,
                }
            })
            .map(|url| {
                ReqwestService {
                    url,
                }
            })
    }
}

#[derive(Debug)]
pub enum Request {
    Get {
        path: String,
    },
}

impl From<()> for Request {
    fn from(_: ()) -> Request {
        Request::Get {
            path: "".to_owned()
        }
    }
}

fn exec_request<TRequest, TResponse>(svc: &ReqwestService, req: TRequest) -> ServiceResult<reqwest::Response> where
    TRequest: Into<Request>,
{
    let url = svc.url.to_owned();
    let req = req.into();

    match req {
        Request::Get { path } => {
            let url = url
                .join(&path)
                .map_err(|error| ServiceError {
                    context: ErrorContext {
                        response: None,
                        error: None,
                    },
                    error: GatewayError::UrlParseFailed(error),
                })?;
            println!("Reqwest Get: {:?}", url);
            reqwest::get(url.as_str())
                .map_err(|error| ServiceError {
                    context: ErrorContext {
                        response: None,
                        error: Some(error),
                    },
                    error: GatewayError::RequestFailed,
                })
        }
    }
}

fn extract_text(mut response: reqwest::Response) -> ServiceResult<(reqwest::Response, String)> {
    match response.text() {
        Ok(text) => Ok((response, text)),
        Err(error) => Err(ServiceError {
            context: ErrorContext {
                response: Some(response),
                error: Some(error),
            },
            error: GatewayError::ReadBodyFailed
        }),
    }
}

fn validate_status((response, text): (reqwest::Response, String)) -> ServiceResult<(reqwest::Response, String)> {
    if response.status().eq(&200) {
        Ok((response, text))
    } else {
        Err (ServiceError {
            context: ErrorContext {
                response: Some(response),
                error: None,
            },
            error: GatewayError::ResultFailed {
                payload: text.to_owned(),
                message: serde_json::from_str::<v1::Message>(&text).map(Message::V1),
            }
        })
    }
}

fn parse_response<TResponse>((response, text): (reqwest::Response, String)) -> ServiceResult<TResponse> where
    TResponse: serde::de::DeserializeOwned + std::fmt::Debug
{
    serde_json::from_str::<TResponse>(&text)
        .map_err(|error| ServiceError {
            context: ErrorContext {
                response: Some(response),
                error: None,
            },
            error: GatewayError::InvalidPayload {
                error,
                payload: text.to_owned(),
                message: serde_json::from_str::<v1::Message>(&text).map(Message::V1),
            }
        })
}

impl Service for ReqwestService {
    type TRequest = Request;
    type TContext = ErrorContext;
    type TPayloadSerdeError = serde_json::error::Error;
    type TMessageSerdeError = serde_json::error::Error;

    fn exec<TRequest, TResponse>(&self, req: TRequest) -> ServiceResult<TResponse> where
        TRequest: Into<Self::TRequest> + std::fmt::Debug,
        TResponse: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        println!("REQWEST\tAPI REQ: [{:?}]", req);
        debug!("REQWEST\tAPI REQ: [{:?}]", req);

        exec_request::<TRequest, TResponse>(self, req)
            .and_then(extract_text)
            .and_then(validate_status)
            .and_then(parse_response)
    }
}

#[cfg(test)]
mod tests {
    // use mockito::{ mock, Matcher };
    use mockito::{ mock };

    use super::{ GatewayError, ReqwestService, Service };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn fail_ctor_with_empty_url() {
        init();
        match ReqwestService::with_url("") {
            Ok (result) => assert!(false, "should have failed empty url but was [{:?}]", result),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::UrlParseFailed(inner) => {
                        let err = format!("{:?}", inner);
                        assert_eq!("RelativeUrlWithoutBase", err);
                    },
                    _ => assert!(false, "expected UrlParseFailed but was [{:?}]", service_error.error),
                }
            }
        }
    }


    #[test]
    fn return_error_for_404() {
        init();
        let mock = mock("GET", "/return_error_for_404")
            .with_status(404)
            .expect(1)
            .create();

        match ReqwestService::with_url("http://www.foo.net/return_error_for_404")
            .and_then(|svc| svc.exec::<(), ()>(()))
        {
            Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::ResultFailed { .. } => {},
                    _ => assert!(false, "expected ResultFailed relaetd error but was [{:?}]", service_error.error),
                }
            },
        }
        mock.assert();
    }

    #[test]
    fn return_error_for_500() {
        init();
        let mock = mock("GET", "/return_error_for_500")
            .with_status(500)
            .expect(1)
            .create();

        match ReqwestService::with_url("http://www.foo.net/return_error_for_500")
            .and_then(|svc| svc.exec::<(), ()>(()))
        {
            Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::ResultFailed { .. } => {},
                    _ => assert!(false, "expected ResultFailed relaetd error but was [{:?}]", service_error.error),
                }
            },
        }
        mock.assert();
    }

    #[test]
    fn return_error_for_invalid_payload() {
        init();
        let mock = mock("GET", "/return_error_for_invalid_payload")
            .with_status(200)
            .with_body("foo=bar")
            .expect(1)
            .create();

        match ReqwestService::with_url("http://www.foo.net/return_error_for_invalid_payload")
            .and_then(|svc| svc.exec::<(), ()>(()))
        {
            Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::InvalidPayload { .. } => {},
                    _ => assert!(false, "expected StatusCode relaetd error but was [{:?}]", service_error.error),
                }
            },
        }
        mock.assert();
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct Temp {
        foo: u16,
    }

    #[test]
    fn parse_payload_successfully() {
        init();
        let mock = mock("GET", "/parse_payload_successfully")
            .with_status(200)
            .with_body(r#"{"foo":10}"#)
            .expect(1)
            .create();

        match ReqwestService::with_url("http://www.foo.net/parse_payload_successfully")
            .and_then(|svc| svc.exec::<(), Temp>(()))
        {
            Ok (result) => assert_eq!(10, result.foo),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::InvalidPayload { message, payload, error } => assert!(false, "expected succful payload parse but was [{:?}] [{:?}] for [{:?}]", message, error, payload),
                    _ => assert!(false, "expected StatusCode relaetd error but was [{:?}]", service_error.error),
                }
            },
        }
        mock.assert();
    }
}