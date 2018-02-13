extern crate futures;
extern crate tokio_io;
extern crate tokio_core;
extern crate bytes;
extern crate net2;
extern crate num_cpus;
extern crate native_tls;
extern crate tokio_tls;
#[macro_use]
extern crate error_chain;

use futures::prelude::*;
use tokio_io::{AsyncRead, AsyncWrite};
use std::net::SocketAddr;
use futures::{Stream, Sink};
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Handle};
use std::sync::Arc;
use std::thread;
use std::io::Read;
use native_tls::{TlsAcceptor, Pkcs12};
use tokio_tls::TlsAcceptorExt;
use bytes::{BufMut, BytesMut};

mod error;
use error::*;
mod codec;
use codec::LengthCodec;

struct LengthProto {
    response_factor: f32,
    tls_acceptor: TlsAcceptor
}

fn main() {
    let mut file = std::fs::File::open("gateway.tests.com.pfx").expect("TLS cert file must be present in current dir");
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12).expect("could not read TLS cert file");
    let pkcs12 = Pkcs12::from_der(&pkcs12, "password").expect("could not load TLS cert");
    let acceptor = TlsAcceptor::builder(pkcs12).unwrap().build().unwrap();

    let workers = num_cpus::get();
    let rep_1_1 = start_length_server("0.0.0.0:21000", workers, &acceptor, 1f32);
    let rep_1_100 = start_length_server("0.0.0.0:21001", workers, &acceptor, 100f32);
    let rep_100_1 = start_length_server("0.0.0.0:21002", workers, &acceptor, 0.01f32);

    println!("started");
    

    rep_1_1.join().unwrap();
    rep_1_100.join().unwrap();
    rep_100_1.join().unwrap();
}

fn start_length_server(addr: &'static str, workers: usize, acceptor: &TlsAcceptor, response_factor: f32) -> std::thread::JoinHandle<()> {
    let acceptor = acceptor.clone();
    std::thread::spawn(move || start_server(addr.parse().unwrap(), workers, move |_| LengthProto { response_factor: 1f32, tls_acceptor: acceptor.clone() }))
}

impl LengthProto {
    pub fn bind<Io: AsyncRead + AsyncWrite + 'static>(self, handle: &Handle, io: Io) {
        let h = handle.clone();
        let start = self.tls_acceptor.accept_async(io)
            .and_then(move |tls| {
                self.bind_length(&h, tls);
                Ok(())
            })
            .map_err(|_| ());
        handle.spawn(start);
    }
    
    fn bind_length<Io: AsyncRead + AsyncWrite + 'static>(self, handle: &Handle, io: Io) {
        let framed = io.framed(LengthCodec::new());
        let handle = handle.clone();
        let start_handle = handle.clone();
        let (tx, rx) = framed.split();
        let transformed = rx.map(move |m| {
            if self.response_factor == 1.0f32 {
                return m;
            }
            let len = m.len();
            let mut fill_len = std::cmp::max(1, (len as f32 * self.response_factor) as usize);
            let mut response = BytesMut::with_capacity(fill_len);
            while fill_len > len {
                response.put_slice(m.as_ref());
                fill_len -= len;
            }
            if fill_len > 0 {
                response.put_slice(&m.as_ref()[0..fill_len]);
            }
            response
            });
        let work = tx.send_all(transformed)
            .map_err(|e| {
                println!("err: {:?}", e);
                e
            })
            .then(|_| Ok(()));
        start_handle.spawn(work);
    }
}

fn start_server<F>(addr: SocketAddr, threads: usize, new_service: F)
    where F: Fn(&Handle) -> LengthProto + Send + Sync + 'static,
{
    let new_service = Arc::new(new_service);
    let threads = (0..threads - 1)
        .map(|i| {
            let new_service = new_service.clone();

            thread::Builder::new()
                .name(format!("worker{}", i))
                .spawn(move || serve(addr, &*new_service))
                .unwrap()
        })
        .collect::<Vec<_>>();

    serve(addr, &*new_service);

    for thread in threads {
        thread.join().unwrap();
    }
}


fn serve<F>(addr: SocketAddr, new_service: F)
where
    F: Fn(&Handle) -> LengthProto,
{
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    //let new_service = new_service(&handle);
    let listener = listener(&addr, &handle).unwrap();

    let server = listener.incoming().for_each(move |(socket, _)| {
        socket.set_nodelay(true).unwrap();
        // Create the service
        let service = new_service(&handle);

        // Bind it!
        service.bind(&handle, socket);

        Ok(())
    });

    core.run(server).unwrap();
}

fn listener(addr: &SocketAddr, handle: &Handle) -> std::io::Result<TcpListener> {
    let listener = match *addr {
        SocketAddr::V4(_) => net2::TcpBuilder::new_v4()?,
        SocketAddr::V6(_) => net2::TcpBuilder::new_v6()?,
    };
    configure_tcp(&listener)?;
    listener.reuse_address(true)?;
    listener.bind(addr)?;
    listener
        .listen(1024)
        .and_then(|l| TcpListener::from_listener(l, addr, handle))
}

#[cfg(unix)]
fn configure_tcp(tcp: &net2::TcpBuilder) -> std::io::Result<()> {
    use net2::unix::*;

    tcp.reuse_port(true)?;

    Ok(())
}

#[cfg(windows)]
fn configure_tcp(_tcp: &net2::TcpBuilder) -> std::io::Result<()> {
    Ok(())
}
