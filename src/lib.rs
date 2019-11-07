#[macro_use]
extern crate log;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;
#[cfg(test)]
extern crate env_logger;
#[cfg(test)]
extern crate mockito;

extern crate gateway;

use std::fmt;

use gateway::{parse_url, Endpoint, Service, ServiceResult};

#[derive(Debug)]
pub enum Error {
    /// An atttempt to append a path to the base url failed to parse
    AppendPathFailed(url::ParseError),
    /// Call to backing service failed
    RequestFailed(reqwest::Error),
    /// Unable to parse api response to extract payload content
    ReadBodyFailed(reqwest::Error),
    /// API returned a failure, such as invalid HTTP status code
    ResultFailed { payload: String },
    /// Api call succeeded, e.g. with 200 OK, but payload did not parse successfully
    InvalidPayload {
        serde_error: serde_json::error::Error,
        payload: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::AppendPathFailed(_err) => write!(f, "Internal Server Error - Invalid Path"),
            Error::RequestFailed(err) => write!(f, "{}", err),
            Error::ReadBodyFailed(err) => write!(f, "{}", err),
            Error::ResultFailed { payload } => write!(f, "Internal Server Error [{}]", payload),
            Error::InvalidPayload { serde_error, payload } => write!(f, "Failed to parse response [{}] because [{}]", payload, serde_error),
        }
    }
}

/// Service implementation using Reqwest for proxying to the backing api(s)
pub struct ReqwestJsonService {
    url: url::Url,
}

impl fmt::Debug for ReqwestJsonService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReqwestJsonService {{ url: {} }}", self.url)
    }
}

impl ReqwestJsonService {
    pub fn with_url(url_str: &str) -> Result<Self, gateway::Error> {
        parse_url(url_str).map(|url| ReqwestJsonService { url })
    }
}

#[derive(Debug)]
pub enum Request {
    Get { path: String },
}

fn build_path(url: url::Url, path: String) -> Result<url::Url, Error> {
    url.join(&path).map_err(Error::AppendPathFailed)
}

fn get(url: url::Url) -> Result<reqwest::Response, Error> {
    reqwest::get(url.as_str()).map_err(Error::RequestFailed)
}

fn exec_request<TRequest>(
    svc: &ReqwestJsonService,
    req: TRequest,
) -> Result<reqwest::Response, Error>
where
    // Result<reqwest::Response, (Error, OptionResult<TError>)> where
    TRequest: Into<Request>,
{
    let url = svc.url.to_owned();
    let req = req.into();
    match req {
        Request::Get { path } => build_path(url, path).and_then(get),
    }
}

fn extract_text(mut response: reqwest::Response) -> Result<String, Error> {
    response.text().map_err(Error::ReadBodyFailed)
}

fn validate_status<TError>(
    status: reqwest::StatusCode,
    text: String,
) -> Result<String, (Error, Option<Result<TError, serde_json::Error>>)>
where
    TError: serde::de::DeserializeOwned + fmt::Debug,
{
    if status.eq(&200) {
        Ok(text)
    } else {
        Err((
            Error::ResultFailed {
                payload: text.to_owned(),
            },
            Some(serde_json::from_str::<TError>(&text)),
        ))
    }
}

fn parse_response<TResponse, TError>(
    text: String,
) -> Result<TResponse, (Error, Option<Result<TError, serde_json::Error>>)>
where
    TResponse: serde::de::DeserializeOwned + std::fmt::Debug,
    TError: serde::de::DeserializeOwned + std::fmt::Debug,
{
    serde_json::from_str::<TResponse>(&text).map_err(|serde_error| {
        (
            Error::InvalidPayload {
                serde_error,
                payload: text.to_owned(),
            },
            Some(serde_json::from_str::<TError>(&text)),
        )
    })
}

impl Service for ReqwestJsonService {
    type TRequestType = Request;
    type TServiceError = Error;
    type TErrorSerde = serde_json::Error;

    fn exec<TRequest>(
        &self,
        req: TRequest,
    ) -> ServiceResult<TRequest, Self::TServiceError, serde_json::Error>
    where
        TRequest: Into<Self::TRequestType> + Endpoint + fmt::Debug,
    {
        println!("REQWEST\tAPI REQ: [{:?}]", req);
        debug!("REQWEST\tAPI REQ: [{:?}]", req);

        // Call the service
        let result = match exec_request::<TRequest>(self, req) {
            Ok(resp) => {
                let status = resp.status();
                // Pull out the body text
                extract_text(resp)
                    .map_err(|err| (err, None))
                    // Fallback to error handling for invlaid status
                    .and_then(|text| validate_status(status, text))
                    // Try to deserialize the body as the expected type
                    .and_then(parse_response)
            }
            Err(err) => Err((err, None)),
        };
        match result {
            Ok(resp) => ServiceResult::Ok(resp),
            Err((svc_err, None)) => ServiceResult::Fail(svc_err, None),
            Err((svc_err, Some(err_result))) => match err_result {
                Ok(err) => ServiceResult::Err(svc_err, err),
                Err(serde_err) => ServiceResult::Fail(svc_err, Some(serde_err)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use mockito::mock;

    use super::{Endpoint, Error, Request, ReqwestJsonService, Service, ServiceResult};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Unit {}

    impl From<Unit> for Request {
        fn from(_: Unit) -> Request {
            Request::Get {
                path: "".to_owned(),
            }
        }
    }

    impl Endpoint for Unit {
        type TResponse = UnitResult;
        type TError = UnitError;
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct UnitResult {}

    #[derive(Debug, Deserialize, Serialize)]
    struct UnitError {}

    #[test]
    fn fail_ctor_with_empty_url() {
        init();
        match ReqwestJsonService::with_url("") {
            Ok(svc) => assert!(false, "should have failed empty url but was [{:?}]", svc),
            Err(error) => match error {
                gateway::Error::UrlParseFailed(inner) => {
                    let err = format!("{:?}", inner);
                    assert_eq!("RelativeUrlWithoutBase", err);
                }
                _ => assert!(false, "expected UrlParseFailed but was [{:?}]", error),
            },
        }
    }

    #[test]
    fn return_error_for_404_with_error_payload() {
        init();
        let mock = mock("GET", "/return_error_for_404")
            .with_status(404)
            .with_body("{}")
            .expect(1)
            .create();

        let svc = ReqwestJsonService::with_url("http://www.foo.net/return_error_for_404").unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok(result) => assert!(
                false,
                "should have detected invalid status but was [{:?}]",
                result
            ),
            ServiceResult::Err(service_error, _api_error) => match service_error {
                Error::ResultFailed { .. } => {}
                _ => assert!(
                    false,
                    "expected ResultFailed relaetd error but was [{:?}]",
                    service_error
                ),
            },
            ServiceResult::Fail(service_error, maybe_api_serde) => assert!(
                false,
                "should have had an api error [{:?}] to parse but was [{:?}]",
                service_error, maybe_api_serde
            ),
        }
        mock.assert();
    }

    #[test]
    fn return_fail_for_404_without_error_payload() {
        init();
        let mock = mock("GET", "/return_error_for_404")
            .with_status(404)
            .expect(1)
            .create();

        let svc = ReqwestJsonService::with_url("http://www.foo.net/return_error_for_404").unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok(result) => assert!(
                false,
                "should have detected invalid status but was [{:?}]",
                result
            ),
            ServiceResult::Err(_service_error, api_error) => assert!(
                false,
                "should not have had an api error to parse but was [{:?}]",
                api_error
            ),
            ServiceResult::Fail(service_error, maybe_api_serde) => {
                match service_error {
                    Error::ResultFailed { .. } => {}
                    _ => assert!(
                        false,
                        "expected ResultFailed relaetd error but was [{:?}]",
                        service_error
                    ),
                }
                assert!(
                    maybe_api_serde.is_some(),
                    "api response should serde error: [{:?}]",
                    maybe_api_serde
                );
            }
        }
        mock.assert();
    }

    #[test]
    fn return_fail_for_500_without_error_payload() {
        init();
        let mock = mock("GET", "/return_error_for_500")
            .with_status(500)
            .expect(1)
            .create();

        let svc = ReqwestJsonService::with_url("http://www.foo.net/return_error_for_500").unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok(result) => assert!(
                false,
                "should have detected invalid status but was [{:?}]",
                result
            ),
            ServiceResult::Err(_service_error, api_error) => assert!(
                false,
                "should not have had an api error to parse but was [{:?}]",
                api_error
            ),
            ServiceResult::Fail(service_error, maybe_api_serde) => {
                match service_error {
                    Error::ResultFailed { .. } => {}
                    _ => assert!(
                        false,
                        "expected ResultFailed relaetd error but was [{:?}]",
                        service_error
                    ),
                }
                assert!(
                    maybe_api_serde.is_some(),
                    "api response should serde error: [{:?}]",
                    maybe_api_serde
                );
            }
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

        let svc =
            ReqwestJsonService::with_url("http://www.foo.net/return_error_for_invalid_payload")
                .unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok(result) => assert!(
                false,
                "should have been valid status with invalid payload and was [{:?}]",
                result
            ),
            ServiceResult::Err(_service_error, api_error) => assert!(
                false,
                "should not have had an api error to parse but was [{:?}]",
                api_error
            ),
            ServiceResult::Fail(service_error, maybe_api_serde) => {
                match service_error {
                    Error::InvalidPayload { .. } => {},
                    _ => assert!(false, "expected InvalidPayload relaetd error but was [{:?}] with error serde [{:?}]", service_error, maybe_api_serde),
                }
                assert!(
                    maybe_api_serde.is_some(),
                    "api response should serde error: [{:?}]",
                    maybe_api_serde
                );
            }
        }
        mock.assert();
    }

    #[test]
    fn return_success_for_valid_status_and_payload() {
        init();
        let mock = mock("GET", "/return_error_for_invalid_payload")
            .with_status(200)
            .with_body("{}")
            .expect(1)
            .create();

        let svc =
            ReqwestJsonService::with_url("http://www.foo.net/return_error_for_invalid_payload")
                .unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok (_) => {},
            ServiceResult::Err (service_error, api_error) => assert!(false, "should not have failed with [{:?}] or had an api error to parse but was [{:?}]", service_error, api_error),
            ServiceResult::Fail (service_error, maybe_api_serde) => assert!(false, "should not have failed with [{:?}] or had an api error to parse but failed with [{:?}]", service_error, maybe_api_serde),
        }
        mock.assert();
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct TempRequest {}

    #[derive(Debug, Serialize, Deserialize)]
    struct TempResponse {
        foo: u16,
    }

    impl From<TempRequest> for Request {
        fn from(_src: TempRequest) -> Request {
            Request::Get {
                path: "".to_owned(),
            }
        }
    }

    impl Endpoint for TempRequest {
        type TResponse = TempResponse;
        type TError = ();
    }

    #[test]
    fn parse_more_complex_payload_successfully() {
        init();
        let mock = mock("GET", "/parse_payload_successfully")
            .with_status(200)
            .with_body(r#"{"foo":10}"#)
            .expect(1)
            .create();

        let svc =
            ReqwestJsonService::with_url("http://www.foo.net/parse_payload_successfully").unwrap();

        match svc.exec(TempRequest {}) {
            ServiceResult::Ok (result) => assert_eq!(10, result.foo),
            ServiceResult::Err (service_error, api_error) => assert!(false, "should not have failed with [{:?}] or had an api error to parse but was [{:?}]", service_error, api_error),
            ServiceResult::Fail (service_error, maybe_api_serde) => assert!(false, "should not have failed with [{:?}] or had an api error to parse but failed with [{:?}]", service_error, maybe_api_serde),
        }
        mock.assert();
    }
}
