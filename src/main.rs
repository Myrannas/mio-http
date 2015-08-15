#![feature(unboxed_closures, fnbox, drain, core)]
extern crate mio;
extern crate bytes;
extern crate core;
#[macro_use]
extern crate log;
extern crate threadpool;

mod request;
mod processor;
mod promises;

use mio::*;
use mio::tcp::*;
use mio::util::Slab;
use bytes::{ByteBuf, MutByteBuf};
use std::io;
use std::mem;
use request::HttpResult;

const SERVER : Token = Token(0);

struct HttpConnection {
    sock: TcpStream,
    buf: Option<ByteBuf>,
    mut_buf: Option<MutByteBuf>,
    token: Option<Token>,
    interest: EventSet,
    http_request: Option<request::HttpRequestBuilder>
}

impl HttpConnection {
    fn new(sock: TcpStream) -> HttpConnection {
        HttpConnection {
            sock: sock,
            buf: None,
            mut_buf: Some(ByteBuf::mut_with_capacity(2048*8)),
            token: None,
            interest: EventSet::hup(),
            http_request: Some(request::HttpRequestBuilder::new())
        }
    }

    fn readable(&mut self, _: &mut EventLoop<HttpHandler>) -> io::Result<()> {
        let mut buf = self.mut_buf.take().unwrap();

        match self.sock.try_read_buf(&mut buf) {
            Ok(None) => {
                panic!("Received readable notification but was unable to read from socket");
            }
            Ok(Some(_)) => {
                //Check if end of request
                let read_buffer = buf.flip();

                match mem::replace(&mut self.http_request, None) {
                    Some(http_request) =>
                        match http_request.parse(read_buffer) {
                            Ok(HttpResult::Http1Incomplete{buffer, request_builder}) => (),
                            _ => ()
                        },
                    None => ()
                }



                // if self.http_request.is_complete() {
                //     let request_token = match & self.token {
                //         &Some(token) => token.clone(),
                //         &None => panic!("No token assigned to port")
                //     };
                //
                //     let mut http_request = request::HttpRequestBuilder::new();
                //     mem::swap(&mut self.http_request, &mut http_request);
                // }
            }
            Err(e) => {
                println!("Error encountered {:?}", e);
            }
        }

        Ok(())
    }
}

struct HttpServer {
    sock: TcpListener,
    conns: Slab<HttpConnection>
}

impl HttpServer {
    fn accept(&mut self, event_loop: &mut EventLoop<HttpHandler>) -> io::Result<()> {
        let sock = self.sock.accept().unwrap().unwrap();
        let conn = HttpConnection::new(sock);
        let tok = self.conns.insert(conn)
            .ok().expect("Could not add connection to slab");

        self.conns[tok].token = Some(tok);
        event_loop.register_opt(&self.conns[tok].sock, tok, EventSet::readable(), PollOpt::edge() | PollOpt::oneshot()).ok().expect("Could not register socket with event loop");

        Ok(())
    }

    fn conn_readable(&mut self, event_loop: &mut EventLoop<HttpHandler>, tok: Token) -> io::Result<()> {
        self.conn(tok).readable(event_loop)
    }

    fn conn<'a>(&'a mut self, tok: Token) -> &'a mut HttpConnection {
        &mut self.conns[tok]
    }
}

struct HttpHandler {
    server: HttpServer
}

impl HttpHandler {
    fn new(srv: TcpListener) -> HttpHandler {
        HttpHandler {
            server: HttpServer {
                sock: srv,
                conns: Slab::new_starting_at(Token(1), 128)
            }
        }
    }
}

impl Handler for HttpHandler {

    type Timeout = ();
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<HttpHandler>, token: Token, events:EventSet) {
        if events.is_readable() {
            match token {
                SERVER => self.server.accept(event_loop).unwrap(),
                i => self.server.conn_readable(event_loop, i).unwrap()
            }
        }
    }
}

fn main() {
    start();
}

fn start() {
    println!("Starting event loop");
    let addr = "127.0.0.1:8080".parse().unwrap();
    let server = TcpListener::bind(&addr).unwrap();
    let mut event_loop = EventLoop::new().unwrap();
    event_loop.register(&server, SERVER).unwrap();

    let mut handler = HttpHandler::new(server);
    event_loop.run(&mut handler).unwrap();
}
