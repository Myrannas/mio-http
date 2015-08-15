use std::convert::AsRef;
use std::collections::HashMap;
use bytes::{Buf, ByteBuf, MutByteBuf};
use std::mem;
use std::error::Error;
use std::convert::From;
use std::string;

#[derive(Debug)]
struct HttpError {
    message: String,
    cause: Option<Box<Error>>
}

pub enum HttpResult {
    Http1Incomplete {buffer: MutByteBuf, request_builder: HttpRequestBuilder},
    Http1Request {buffer: ByteBuf, request: HttpRequest},
    Http2Upgrade {buffer: ByteBuf, request: HttpRequest},
}

impl HttpError {
    fn new(message: String) -> HttpError {
        HttpError {
            message: message,
            cause: None
        }
    }

    fn with_cause(message: String, err: Box<Error>) -> HttpError {
        HttpError {
            message: message,
            cause: Some(err)
        }
    }
}

impl PartialEq for HttpError {
    fn eq(&self, other: &HttpError) -> bool {
        other.message == self.message
    }
}

impl From<string::FromUtf8Error> for HttpError {
    fn from(err: string::FromUtf8Error) -> HttpError {
        HttpError::with_cause(String::from("Error parsing string"), Box::new(err))
    }
}

#[derive(Debug, PartialEq)]
enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    OPTIONS,
    HTTP2
}

impl HttpMethod {
    fn parse(value :&str) -> Result<HttpMethod, HttpError> {
        match value.to_uppercase().as_ref() {
            "GET" => Ok(HttpMethod::GET),
            "POST" => Ok(HttpMethod::POST),
            "PUT" => Ok(HttpMethod::PUT),
            "DELETE" => Ok(HttpMethod::DELETE),
            "OPTIONS" => Ok(HttpMethod::OPTIONS),
            "PRI" => Ok(HttpMethod::HTTP2),
            other => {
                Err(HttpError::new(format!("Unrecognised method {}", other)))
            }
        }
    }
}

#[derive(PartialEq, Debug)]
enum ParserStates {

    //Initial request states
    Verb,
    Path,
    Version,

    //Header parsing states
    HeaderTitle,
    HeaderContent{title: String},
    HeadersNewLine{title: String},
    EndHeaders,

    // Section complete states
    Complete
}

#[derive(Debug)]
pub struct HttpRequestBuilder{
    method: Result<HttpMethod, HttpError>,
    path: Option<String>,
    version: Option<String>,
    headers: HashMap<String, Vec<String>>,
    state: ParserStates,
    temporary_data: Vec<u8>
}

struct HttpRequest {
    method: HttpMethod,
    path: String,
    headers: HashMap<String, Vec<String>>
}

impl  HttpRequestBuilder {
    pub fn new() -> HttpRequestBuilder {
        HttpRequestBuilder {
            method : Err(HttpError::new(String::from("Method not declared"))),
            path: None,
            version: None,
            headers: HashMap::new(),
            state: ParserStates::Verb,
            temporary_data: Vec::new()
        }
    }

    pub fn build(self) -> Result<HttpRequest, HttpError> {
        let method = try!(self.method);
        let path = try!(self.path.ok_or(HttpError::new(format!("Path not parsed"))));

        Ok(HttpRequest {
            method: method,
            path: path,
            headers: self.headers
        })
    }

    fn read_value(&mut self, buffer: &mut ByteBuf, length: usize) -> Result<String, HttpError> {
        let (mut data, start) = match self.temporary_data.len() {
            0 => (Vec::with_capacity(length), 0),
            _ => {
                println!("Using leftover data");
                let mut temp = mem::replace(&mut self.temporary_data, Vec::new());
                let start = temp.len();
                temp.reserve(length);
                (temp, start)
            }
        };

        unsafe{ data.set_len(start + length); }
        buffer.reset();
        buffer.read_slice(&mut data[start .. (start + length)]);
        buffer.advance(1);
        buffer.mark();
        match String::from_utf8(data) {
            Ok(data) => Ok(String::from(data.trim())),
            Err(err) => Err(HttpError::from(err))
        }
    }

    pub fn parse(mut self, mut buffer: ByteBuf) -> Result<HttpResult, HttpError> {
        let mut state_length = 0;
        buffer.mark();

        while let Some(character) = buffer.read_byte() {
            let state = mem::replace(&mut self.state, ParserStates::Complete);
            println!("({:?}, {:?})",state, character as char);
            let (next_state, next_length) = match (state, character as char) {
                (ParserStates::Verb, ' ') => {
                    let verb = try!(self.read_value(&mut buffer, state_length));
                    self.method = HttpMethod::parse(& verb);
                    (ParserStates::Path, 0)
                },
                (ParserStates::Path, ' ') => {
                    self.path = Some(try!(self.read_value(&mut buffer, state_length)));
                    (ParserStates::Version, 0)
                },
                (ParserStates::Version, '\n') => {
                    self.version = Some(try!(self.read_value(&mut buffer, state_length)));
                    (ParserStates::HeaderTitle, 0)
                },
                (ParserStates::HeaderTitle, '\n') => (ParserStates::Complete, 1),
                (ParserStates::HeaderTitle, ':') => {
                    let header_title = try!(self.read_value(&mut buffer, state_length)).to_uppercase();
                    (ParserStates::HeaderContent{title: header_title}, 0)
                },
                (ParserStates::HeaderContent{title}, '\n') =>
                    (ParserStates::HeadersNewLine{title: title}, state_length + 1),
                (ParserStates::HeadersNewLine{title}, ' ') | (ParserStates::HeadersNewLine{title}, '\t') => {
                    (ParserStates::HeaderContent{title: title}, state_length + 2)
                },
                (ParserStates::HeadersNewLine{title}, '\n') => {
                    let header_value = vec![try!(self.read_value(&mut buffer, state_length))];
                    self.headers.insert(title, header_value);
                    (ParserStates::Complete, 0)
                },
                (ParserStates::HeadersNewLine{title}, '\r') => {
                    let header_value = vec![try!(self.read_value(&mut buffer, state_length))];
                    self.headers.insert(title, header_value);
                    (ParserStates::EndHeaders, 0)
                },
                (ParserStates::HeadersNewLine{title}, _) => {
                    let header_value = vec![try!(self.read_value(&mut buffer, state_length - 1))];
                    self.headers.insert(title, header_value);
                    (ParserStates::HeaderTitle, 0)
                },
                (ParserStates::EndHeaders, '\n') => (ParserStates::Complete, 0),
                (ParserStates::EndHeaders, character) => panic!("Malformed headers {:?}", character),
                (state, _) => (state, state_length + 1)
            };

            if next_state == ParserStates::Complete {
                match self.build() {
                    Ok(request) =>
                        match request.method {
                            HttpMethod::HTTP2 => return Ok(HttpResult::Http2Upgrade{buffer: buffer, request: request}),
                            _ => return Ok(HttpResult::Http1Request{buffer: buffer, request: request})
                        },

                    Err(err) =>
                        return Err(err)
                }
            }

            mem::replace(&mut self.state, next_state);
            state_length = next_length;
        }

        if state_length>0{
            println!("Storing {} leftover bytes", state_length);
            let mut temporary_data = &mut self.temporary_data;
            temporary_data.reserve(state_length);

            let current_length = temporary_data.len();
            unsafe{temporary_data.set_len(current_length + state_length);}
            buffer.reset();
            buffer.read_slice(&mut temporary_data);
        }

        Ok(HttpResult::Http1Incomplete{buffer: buffer.flip(), request_builder: self})
    }
}

impl HttpRequest {

}

#[cfg(test)]
mod tests {
    use bytes::ByteBuf;
    use super::HttpRequestBuilder;
    use super::HttpMethod;
    use super::HttpResult;

    #[test]
    fn test_http_request_builder() {
        let buffer = ByteBuf::from_slice("GET / HTTP\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();

        match request_builder.parse(buffer) {
            Ok(HttpResult::Http1Incomplete{request_builder, ..}) => {
                assert_eq!(Ok(HttpMethod::GET), request_builder.method);
                assert_eq!("/", request_builder.path.unwrap());
                assert_eq!("HTTP", request_builder.version.unwrap());
            },
            _ => panic!("Expected Http1Incomplete")
        }
    }

    #[test]
    fn test_http_request_builder_post() {
        let buffer = ByteBuf::from_slice("POST / HTTP\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer) {
            Ok(HttpResult::Http1Incomplete{request_builder, ..}) => {
                assert_eq!(Ok(HttpMethod::POST), request_builder.method);
                assert_eq!("/", request_builder.path.unwrap());
                assert_eq!("HTTP", request_builder.version.unwrap());
            }
            _ => panic!("Expected Http1Incomplete")
        }
    }

    #[test]
    fn test_http_request_builder_complete_state() {
        let mut buffer = ByteBuf::mut_with_capacity(2048);
        buffer.write_slice("GET ".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer.flip()) {
            Ok(HttpResult::Http1Incomplete{mut buffer, request_builder}) => {
                buffer.write_slice("/ HTTP\n".as_bytes());
                match request_builder.parse(buffer.flip()) {
                    Ok(HttpResult::Http1Incomplete{ request_builder, .. }) => {
                        assert_eq!(Ok(HttpMethod::GET), request_builder.method);
                        assert_eq!("/", request_builder.path.unwrap());
                        assert_eq!("HTTP", request_builder.version.unwrap());
                    }

                    _ => panic!("Expected Http1Incomplete")
                }
            },

            _ => panic!("Expected Http1Incomplete")
        }
    }

    #[test]
    fn test_http_request_builder_incomplete_state() {
        let mut buffer = ByteBuf::mut_with_capacity(2048);
        buffer.write_slice("GET".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer.flip()) {
            Ok(HttpResult::Http1Incomplete{mut buffer, request_builder}) => {

                buffer.write_slice(" / H".as_bytes());
                match request_builder.parse(buffer.flip()) {
                    Ok(HttpResult::Http1Incomplete{mut buffer, request_builder}) => {

                        buffer.write_slice("TTP\r\n".as_bytes());
                        match request_builder.parse(buffer.flip()) {
                            Ok(HttpResult::Http1Incomplete{request_builder, ..}) => {

                                assert_eq!(Ok(HttpMethod::GET), request_builder.method);
                                assert_eq!("/", request_builder.path.unwrap());
                                assert_eq!("HTTP", request_builder.version.unwrap());
                            }

                            _ => panic!("Expected Http1Incomplete")
                        }
                    }

                    _ => panic!("Expected Http1Incomplete")
                }
            }

            _ => panic!("Expected Http1Incomplete")
        }
    }

    #[test]
    fn test_http_request_builder_header() {
        let buffer = ByteBuf::from_slice("GET / HTTP 1.1\r\nContent-Type:   application/json\n\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer) {
            Ok(HttpResult::Http1Request{request, ..}) => assert_eq!(request.headers["CONTENT-TYPE"], vec!["application/json"]),
            _ => panic!("Expected Http1Request")
        }
    }

    #[test]
    fn test_http_request_builder_header_return() {
        let buffer = ByteBuf::from_slice("GET / HTTP 1.1\nContent-Type:   application/json\n\r\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer) {
            Ok(HttpResult::Http1Request{request, ..}) => assert_eq!(request.headers["CONTENT-TYPE"], vec!["application/json"]),
            _ => panic!("Expected Http1Request")
        }
    }

    #[test]
    fn test_http_request_builder_two_headers() {
        let buffer = ByteBuf::from_slice("GET / HTTP 1.1\nContent-Type:   application/json\nContent-Length:128\r\n\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer) {
            Ok(HttpResult::Http1Request{request, ..}) => {
                assert_eq!(request.headers["CONTENT-TYPE"], vec!["application/json"]);
                assert_eq!(request.headers["CONTENT-LENGTH"], vec!["128"]);
            }

            _ => panic!("Expected Http1Request")
        }
    }

    #[test]
    fn test_http_request_builder_http2_upgrade() {
        let buffer = ByteBuf::from_slice("PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".as_bytes());
        let request_builder = HttpRequestBuilder::new();
        match request_builder.parse(buffer) {
            Ok(HttpResult::Http2Upgrade{..}) => (),
            _ => panic!("Expected Http2Upgrade")
        }
    }
}
