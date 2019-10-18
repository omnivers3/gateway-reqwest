#[macro_use]
extern crate log;
extern crate reqwest;
extern crate serde;
extern crate serde_json;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;
#[cfg(test)]
extern crate mockito;
#[cfg(test)]
extern crate env_logger;

extern crate gateway;

use std::fmt;
use std::marker::PhantomData;

use gateway::{ parse_url, Message, Request, Service, ServiceError };
use gateway::contracts::{ v1 };

pub type GatewayError = gateway::Error<serde_json::error::Error, serde_json::error::Error>;

#[derive(Debug, Default)]
/// Captures reqwest metadata for diagnosing issues with calls
pub struct ErrorContext {
    response: Option<reqwest::Response>,
    error: Option<reqwest::Error>,
}

/// Service implementation using Reqwest for proxying to the backing api(s)
pub struct ReqwestService<TResult> {
    _result: PhantomData<TResult>,
    url: url::Url,
}

impl<TResult> fmt::Debug for ReqwestService<TResult> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReqwestService {{ url: {} }}", self.url)
    }
}

impl<TResult> ReqwestService<TResult> {
    pub fn with_url(url_str: &str) -> Result<Self, ServiceError<ErrorContext, GatewayError>> {
        parse_url(url_str)
            .map_err(|error| {
                ServiceError {
                    context: Default::default(),
                    error,
                }
            })
            .map(|url| {
                ReqwestService {
                    _result: PhantomData,
                    url,
                }
            })
    }
}

impl<TResult> Service<TResult> for ReqwestService<TResult> {
    type TContext = ErrorContext;
    type TPayloadSerdeError = serde_json::error::Error;
    type TMessageSerdeError = serde_json::error::Error;

    fn get(&self, req: Request) -> Result<TResult, ServiceError<ErrorContext, GatewayError>> where
        TResult: serde::de::DeserializeOwned+ std::fmt::Debug,
    {
        debug!("REQWEST\tAPI REQ: [{:?}]", req);

        let uri: &str = match req {
            Request::Unit => {
                self.url.as_str()
            }
        };
        
        // Call the configured url
        let mut response = reqwest::get(uri)
            .map_err(|error| ServiceError {
                context: ErrorContext {
                    response: None,
                    error: Some(error),
                },
                error: GatewayError::RequestFailed,
            })?;
        // Try to extract the response text to parse
        let text_result = response.text();
        if let Err (error) = text_result {
            return Err (ServiceError {
                context: ErrorContext {
                    response: Some(response),
                    error: Some(error),
                },
                error: GatewayError::ReadBodyFailed
            })
        }
        let text = text_result.unwrap();
        // Ensure only 200 OK responses return success
        if !response.status().eq(&200) {
            return Err (ServiceError {
                context: ErrorContext {
                    response: Some(response),
                    error: None,
                },
                error: GatewayError::ResultFailed {
                    message: serde_json::from_str::<v1::Message>(&text).map(Message::V1),
                }
            })
        }
        // Try to parse the payload from the body text
        serde_json::from_str::<TResult>(&text)
            .map_err(|error| ServiceError {
                context: ErrorContext {
                    response: Some(response),
                    error: None,
                },
                error: GatewayError::InvalidPayload {
                    error,
                    message: serde_json::from_str::<v1::Message>(&text).map(Message::V1),
                }
            })
    }
}

#[cfg(test)]
mod tests {
    // use mockito::{ mock, Matcher };
    use mockito::{ mock };

    use super::{ GatewayError, Request, ReqwestService, Service };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn fail_ctor_with_empty_url() {
        init();
        match ReqwestService::<()>::with_url("") {
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

        match ReqwestService::<()>::with_url("http://www.foo.net/return_error_for_404")
            .and_then(|svc| svc.get(Request::Unit))
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

        match ReqwestService::<()>::with_url("http://www.foo.net/return_error_for_500")
            .and_then(|svc| svc.get(Request::Unit))
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

        match ReqwestService::<()>::with_url("http://www.foo.net/return_error_for_invalid_payload")
            .and_then(|svc| svc.get(Request::Unit))
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

        match ReqwestService::<Temp>::with_url("http://www.foo.net/parse_payload_successfully")
            .and_then(|svc| svc.get(Request::Unit))
        {
            Ok (result) => assert_eq!(10, result.foo),
            Err (service_error) => {
                match service_error.error {
                    GatewayError::InvalidPayload { message, error } => assert!(false, "expected succful payload parse but was [{:?}] [{:?}]", message, error),
                    _ => assert!(false, "expected StatusCode relaetd error but was [{:?}]", service_error.error),
                }
            },
        }
        mock.assert();
    }
}