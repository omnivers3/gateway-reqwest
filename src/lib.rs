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

use std::fmt;

// use gateway::{ parse_url, Response, Service, ServiceError };
use gateway::{ parse_url, Endpoint, Service, ServiceResult };


#[derive(Debug, Default)]
/// Captures reqwest metadata for diagnosing issues with calls
pub struct ErrorContext {
    response: Option<reqwest::Response>,
    internal_error: Option<reqwest::Error>,
}

#[derive(Debug)]
pub enum Error {
    // /// Standard setup error calling gateway utility functions
    // GatewayError(gateway::Error),
    /// An atttempt to append a path to the base url failed to parse
    AppendPathFailed(url::ParseError),
    /// Call to backing service failed
    RequestFailed(reqwest::Error),
    /// Unable to parse api response to extract payload content
    ReadBodyFailed(reqwest::Error),
    /// API returned a failure, such as invalid HTTP status code
    ResultFailed {
        payload: String,
        // response_error: Result<TError, serde_json::error::Error>,
    },
    /// Api call succeeded, e.g. with 200 OK, but payload did not parse successfully
    InvalidPayload {
        serde_error: serde_json::error::Error,
        payload: String,
        // response_error: Result<TError, serde_json::error::Error>,
    }, 
}

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
    pub fn with_url(url_str: &str) -> Result<Self, gateway::Error> {
        parse_url(url_str)
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

fn build_path(url: url::Url, path: String) -> Result<url::Url, Error> {
    url
        .join(&path)
        .map_err(Error::AppendPathFailed)
}

fn get(url: url::Url) -> Result<reqwest::Response, Error> {
    reqwest::get(url.as_str())
        .map_err(Error::RequestFailed)
}

fn exec_request<TRequest>(svc: &ReqwestService, req: TRequest) -> Result<reqwest::Response, Error> where// Result<reqwest::Response, (Error, OptionResult<TError>)> where
    TRequest: Into<Request>,
{
    let url = svc.url.to_owned();
    let req = req.into();
    match req {
        Request::Get { path } => {
            build_path(url, path)
                .and_then(get)
        }
    }
}

fn extract_text(mut response: reqwest::Response) -> Result<String, Error> {
    response
        .text()
        .map_err(Error::ReadBodyFailed)
}

fn validate_status<TError>(status: reqwest::StatusCode, text: String) -> Result<String, (Error, Option<Result<TError, serde_json::Error>>)> where
    TError: serde::de::DeserializeOwned + fmt::Debug,
{
    // if response.status().eq(&200) {
    if status.eq(&200) {
        Ok(text)
    } else {
        Err((
            Error::ResultFailed {
                payload: text.to_owned(),
                // response_error: serde_json::from_str::<TError>(&text),
            },
            Some(serde_json::from_str::<TError>(&text)),
        ))
    }
}

fn parse_response<TResponse, TError>(text: String) -> Result<TResponse, (Error, Option<Result<TError, serde_json::Error>>)> where
    TResponse: serde::de::DeserializeOwned + std::fmt::Debug,
    TError: serde::de::DeserializeOwned + std::fmt::Debug,
{
    serde_json::from_str::<TResponse>(&text)
        .map_err(|serde_error| (
            Error::InvalidPayload {
                serde_error,
                payload: text.to_owned(),
                // response_error: serde_json::from_str::<TError>(&text),
            },
            Some(serde_json::from_str::<TError>(&text)),
        ))
}


impl Service for ReqwestService {
    type TRequestType = Request;
    type TServiceError = Error;
    type TErrorSerde = serde_json::Error;
    // type TError = Error<<Self::TRequestType as gateway::Endpoint>::TError>;

    // fn exec<TRequest>(&self, req: TRequest) -> Result<TRequest::TResponse, (Self::TServiceError, <TRequest as Endpoint>::TError)> where
    fn exec<TRequest>(&self, req: TRequest) -> ServiceResult<TRequest, Self::TServiceError, serde_json::Error> where
        TRequest: Into<Self::TRequestType> + Endpoint + fmt::Debug,
        // <TRequest as gateway::Endpoint>::TError: fmt::Debug + serde::de::DeserializeOwned,
    {
        println!("REQWEST\tAPI REQ: [{:?}]", req);
        debug!("REQWEST\tAPI REQ: [{:?}]", req);

        let result = match exec_request::<TRequest>(self, req)
            // .map_err(|err| (err, None))
        {
            Ok (resp) => {
                let status = resp.status();
                extract_text(resp)
                    .map_err(|err| (err, None))
                    .and_then(|text| validate_status(status, text))
                    .and_then(parse_response)
            },
            Err (err) => Err((err, None)),
        };
        // let result: Result<TRequest::TResponse, (Error, Option<Result<TRequest::TError, serde_json::Error>>)> = exec_request::<TRequest>(self, req)
        //     .map_err(|err| (err, None))
        //     .and_then(|mut resp| {
        //         extract_text(resp)
        //             .map_err(|err| (err, None))
        //             .and_then(|text| validate_status(resp.status(), text))
        //             .and_then(parse_response)
        //     });
        match result {
            Ok (resp) => ServiceResult::Ok (resp),
            Err ((svc_err, None)) => ServiceResult::Fail (svc_err, None),
            Err ((svc_err, Some(err_result))) => match err_result {
                Ok (err) => ServiceResult::Err (svc_err, err),
                Err (serde_err) => ServiceResult::Fail (svc_err, Some(serde_err)),
            }
        }
        // result
        //     .map(ServiceResult::Ok)
        //     .map_err(|(svc_err, error_parse)| {
        //         match error_parse {
        //             None => ServiceResult::Fail (svc_err, None),
        //             Some (err_result) => match err_result {
        //                 Ok (err) => ServiceResult::Err (svc_err, err),
        //                 Err (serde_err) => ServiceResult::Fail (svc_err, Some(serde_err)),
        //             }
        //         }
        //     })
        // match exec_request::<TRequest>(self, req)
        //     .map_err(|err| (err, None))
        //     .and_then(|resp| {
        //         extract_text(resp)
        //             .map_err(|err| (err, None))
        //             .and_then(|text| validate_status(resp.status(), text))
        //             .and_then(parse_response)
        //     }) {
        //     Ok (resp) => ServiceResult::Ok (resp),
        //     Err ((svc_err, None)) => ServiceResult::Fail (svc_err, None),
        //     Err ((svc_err, Some(err_result))) => match err_result {
        //         Ok (err) => ServiceResult::Err (svc_err, err),
        //         Err (serde_err) => ServiceResult::Fail (svc_err, Some(serde_err)),
        //     }
        // }
        // .and_then(validate_status)

        // match 
        // let req_result = ;
        // match req_result {
        //     ServiceResult::Err (_, _) => return req_result,
        //     ServiceResult::Fail (_, _) => return req_result,
        //     ServiceResult::Ok (_) => {},
        // }
            // .map_err(|err| Error::?;//.unwrap();
        // let text = extract_text(req)?;
            // .and_then(|req| {
            //     extract_text(req)
            //         .and_then(|text| validate_status((req.status(), text)))
                    
            // })
            // .and_then(extract_text)
            // .and_then(validate_status)

        // Err(Error::)
    }
}

#[cfg(test)]
mod tests {
    // use mockito::{ mock, Matcher };
    use mockito::{ mock };

    // use super::{ Error, Request, Response, ReqwestService, Service };
    use super::{ Endpoint, Error, Request, ReqwestService, Service, ServiceResult };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Unit {}

    impl From<Unit> for Request {
        fn from(_: Unit) -> Request {
            Request::Get {
                path: "".to_owned()
            }
        }
    }

    impl Endpoint for Unit {
        type TResponse = UnitResult;
        type TError = UnitError;
        // type TErrorSerde = serde_json::Error;
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct UnitResult {}

    #[derive(Debug, Deserialize, Serialize)]
    struct UnitError {}

    // impl Response for Unit {
    //     type TResponse = UnitResult;
    //     type TError = UnitError;
    // }

    #[test]
    fn fail_ctor_with_empty_url() {
        init();
        match ReqwestService::with_url("") {
            Ok (svc) => assert!(false, "should have failed empty url but was [{:?}]", svc),
            Err (error) => {
                match error {
                    gateway::Error::UrlParseFailed(inner) => {
                        let err = format!("{:?}", inner);
                        assert_eq!("RelativeUrlWithoutBase", err);
                    },
                    _ => assert!(false, "expected UrlParseFailed but was [{:?}]", error),
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
        
        let svc = ReqwestService::with_url("http://www.foo.net/return_error_for_404").unwrap();

        match svc.exec(Unit {}) {
            ServiceResult::Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
            ServiceResult::Err (_service_error, api_error) => assert!(false, "should not have had an api error to parse but was [{:?}]", api_error),
            ServiceResult::Fail (service_error, maybe_api_serde) => {
                match service_error {
                    Error::ResultFailed { .. } => {},
                    _ => assert!(false, "expected ResultFailed relaetd error but was [{:?}]", service_error),
                }
                assert!(maybe_api_serde.is_none(), "api response serde error: [{:?}]", maybe_api_serde);
            },

        }
        mock.assert();
    }

    // #[test]
    // fn return_error_for_500() {
    //     init();
    //     let mock = mock("GET", "/return_error_for_500")
    //         .with_status(500)
    //         .expect(1)
    //         .create();

    //     let svc = ReqwestService::with_url("http://www.foo.net/return_error_for_500").unwrap();

    //     match svc.exec(Unit {}) {
    //         Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
    //         Err (service_error) => {
    //             match service_error.error {
    //                 gateway::Error::ServiceError(Error::ResultFailed { .. }) => {},
    //                 _ => assert!(false, "expected ResultFailed relaetd error but was [{:?}]", service_error.error),
    //             }
    //         },
    //     }
    //     mock.assert();
    // }

    // #[test]
    // fn return_error_for_invalid_payload() {
    //     init();
    //     let mock = mock("GET", "/return_error_for_invalid_payload")
    //         .with_status(200)
    //         .with_body("foo=bar")
    //         .expect(1)
    //         .create();

    //     let svc = ReqwestService::with_url("http://www.foo.net/return_error_for_invalid_payload").unwrap();

    //     match svc.exec(Unit {}) {
    //         Ok (result) => assert!(false, "should have detected invalid status but was [{:?}]", result),
    //         Err (service_error) => {
    //             match service_error.error {
    //                 gateway::Error::ServiceError(Error::InvalidPayload { .. }) => {},
    //                 _ => assert!(false, "expected StatusCode relaetd error but was [{:?}]", service_error.error),
    //             }
    //         },
    //     }
    //     mock.assert();
    // }

    // #[derive(Debug, Serialize, Deserialize)]
    // struct TempRequest {}

    // #[derive(Debug, Serialize, Deserialize)]
    // struct TempResponse {
    //     foo: u16,
    // }

    // impl From<TempRequest> for Request {
    //     fn from(src: TempRequest) -> Request {
    //         Request::Get { path: "".to_owned() }
    //     }
    // }

    // // impl Response for TempRequest {
    // //     type TResponse = TempResponse;
    // //     type TError = ();
    // // }

    // #[test]
    // fn parse_payload_successfully() {
    //     init();
    //     let mock = mock("GET", "/parse_payload_successfully")
    //         .with_status(200)
    //         .with_body(r#"{"foo":10}"#)
    //         .expect(1)
    //         .create();

    //     let svc = ReqwestService::with_url("http://www.foo.net/parse_payload_successfully");

    //     match svc.exec(TempRequest {}) {
    //         Ok (result) => assert_eq!(10, result.foo),
    //         Err (service_error) => {
    //             match service_error.error {
    //                 gateway::Error::ServiceError(Error::InvalidPayload { serde_error, payload, response_error }) => assert!(false, "expected succful payload parse but was [{:?}] [{:?}] for [{:?}]", serde_error, response_error, payload),
    //                 _ => assert!(false, "expected StatusCode relaetd error but was [{:?}]", service_error.error),
    //             }
    //         },
    //     }
    //     mock.assert();
    // }
}



// fn build_path(url: url::Url, path: String) -> ServiceResult<url::Url, url::ParseError> {
//     url
//         .join(&path)
//         .map_err(|error| ServiceError {
//             context: ErrorContext {
//                 response: None,
//                 internal_error: None,
//             },
//             error: gateway::Error::UrlParseFailed(error),
//         })
// }

// fn get(url: url::Url) -> ServiceResult<reqwest::Response, reqwest::Error> {
//     reqwest::get(url.as_str())
//         .map_err(|error| ServiceError {
//             context: ErrorContext {
//                 response: None,
//                 internal_error: Some(error),
//             },
//             error: gateway::Error::ServiceError(Error::RequestFailed),
//         })
// }

// fn exec_request<TRequest>(svc: &ReqwestService, req: TRequest) -> ServiceResult<reqwest::Response, Error<TRequest::TError>> where
//     TRequest: Into<Request> + Response,
//     // for<'de> <TRequest as gateway::Response>::TError: serde::Deserialize<'de>,
// {
//     let url = svc.url.to_owned();
//     let req = req.into();

//     match req {
//         Request::Get { path } => {
//             build_path(url, path)
//                 .and_then(get)
//             // let url = url
//             //     .join(&path)
//             //     .map_err(|error| ServiceError {
//             //         context: ErrorContext {
//             //             response: None,
//             //             internal_error: None,
//             //         },
//             //         error: gateway::Error::UrlParseFailed(error),
//             //     })?;
//             // println!("Reqwest Get: {:?}", url);
//             // reqwest::get(url.as_str())
//             //     .map_err(|error| ServiceError {
//             //         context: ErrorContext {
//             //             response: None,
//             //             internal_error: Some(error),
//             //         },
//             //         error: gateway::Error::ServiceError(Error::RequestFailed),
//             //     })
//         }
//     }
// }


// pub trait Endpoint {}

// pub struct Foo {}

// impl Endpoint for Foo {}

// pub trait Service {
//     fn endpoint(&self) -> impl Endpoint;
// }

// impl Service for ReqwestService {
//     fn endpoint(&self) -> impl Endpoint {

//     }
// }

// impl Service for ReqwestService {
//     type TRequest = Request;
//     type TContext = ErrorContext;

//     fn exec<TRequest>(&self, req: TRequest) -> ServiceResult<TRequest::TResponse, TRequest::TError> where
//         TRequest: Into<Self::TRequest> + Response + std::fmt::Debug,
//         // for<'de> <TRequest as gateway::Response>::TError: serde::Deserialize<'de>,
//         // TResponse: serde::de::DeserializeOwned + std::fmt::Debug,
//     {
//         println!("REQWEST\tAPI REQ: [{:?}]", req);
//         debug!("REQWEST\tAPI REQ: [{:?}]", req);

//         let result = exec_request::<TRequest>(self, req);
//             // .and_then(extract_text)
//             // .and_then(validate_status)
//             // .and_then(parse_response)

//         result
//     }
// }


// fn validate_status<TError>((response, text): (reqwest::Response, String)) -> ServiceResult<(reqwest::Response, String), TError> where
//     TError: serde::de::DeserializeOwned + std::fmt::Debug,
// {
//     if response.status().eq(&200) {
//         Ok((response, text))
//     } else {
//         Err (ServiceError {
//             context: ErrorContext {
//                 response: Some(response),
//                 internal_error: None,
//             },
//             error: gateway::Error::ServiceError(Error::ResultFailed {
//                 payload: text.to_owned(),
//                 // serde_error: serde_json::from_str::<v1::Message>(&text).map(Message::V1),
//                 response_error: serde_json::from_str::<TError>(&text)//.map(Message::V1),
//             })
//         })
//     }
// }
