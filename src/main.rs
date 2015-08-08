extern crate mio;
extern crate bytes;

use mio::*;
use mio::tcp::*;
use mio::util::Slab;
use bytes::{ByteBuf, MutByteBuf, SliceBuf};
use std::io;

const SERVER : Token = Token(0);

enum State {
    Init,
    Headers,
    Body
}

struct Header {
    name: String,
    content: String
}

struct HttpPacket {
    request_line: String,
    headers: Vec<Header>,
    body: Option<String>
}

struct HttpConnection {
    sock: TcpStream,
    buf: Option<ByteBuf>,
    mut_buf: Option<MutByteBuf>,
    token: Option<Token>,
    interest: EventSet
}

impl HttpConnection {
    fn new(sock: TcpStream) -> HttpConnection {
        HttpConnection {
            sock: sock,
            buf: None,
            mut_buf: Some(ByteBuf::mut_with_capacity(2048*8)),
            token: None,
            interest: EventSet::hup()
        }
    }

    fn readable(&mut self, event_loop: &mut EventLoop<HttpHandler>) -> io::Result<()> {
        let mut buf = self.mut_buf.take().unwrap();

        match self.sock.try_read_buf(&mut buf) {
            Ok(None) => {
                panic!("Received readable notification but was unable to read from socket");
            }
            Ok(Some(r)) => {
                println!("CONN: we read {} bytes!", r);
                //Check if end of request
                let mut read_buffer = buf.flip();
                let remaining = read_buffer.remaining();
                read_buffer.mark();
                let mut method : [u8;3] = [0;3];
                read_buffer.read_slice(&mut method);

                if std::str::from_utf8(&method) == Ok("GET") {
                    println!("Beginning get")
                } else {
                    panic!("Unsupported method encountered - {:?}", method)
                }

                self.mut_buf = Some(read_buffer.flip());
            }
            Err(e) => {
                println!("Error encountered {:?}", e);
            }
        }

        Ok(())
    }

    fn process(buffer: &ByteBuf) -> Option<()>{
        let mut length = 0u32;
        let mut state = 0;

        Some(())
    }

    fn read_line(buffer: &mut ByteBuf) -> Option<String> {
        let mut length = 0;

        loop {
            match buffer.read_byte() {
                Some(0x0D) => (),
                Some(0x0A) => {
                    return None;
                },
                Some(_) => length+=1,
                None => return None
            }
        }
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
        println!("Reading from connection");
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
