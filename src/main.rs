#![feature(proc_macro, conservative_impl_trait, generators, vec_resize_default, integer_atomics)]

extern crate futures_await as futures;
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

mod error;
use error::*;
mod codec;
use codec::LengthCodec;

struct LengthProto {
    tls_acceptor: TlsAcceptor
}

fn main() {
    println!("starting");

    let mut file = std::fs::File::open("gateway.tests.com.pfx").expect("TLS cert file must be present in current dir");
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12).expect("could not read TLS cert file");
    let pkcs12 = Pkcs12::from_der(&pkcs12, "password").expect("could not load TLS cert");
    let acceptor = TlsAcceptor::builder(pkcs12).unwrap().build().unwrap();

    let addr = "0.0.0.0:10002".parse().unwrap();
    let mqtts_thread = std::thread::spawn(move || {
        with_handle(addr, num_cpus::get(), move |_| {
            LengthProto { tls_acceptor: acceptor.clone() }
        });
    });

    mqtts_thread.join().unwrap();
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
        let work = tx.send_all(rx)
            .map_err(|e| {
                println!("err: {:?}", e);
                e
            })
            .then(|_| Ok(()));
        // let work = rx.forward(tx)
        //     .map_err(|e| {
        //         println!("err: {:?}", e);
        //         e
        //     })
        //     .then(|_| Ok(()));
        start_handle.spawn(work);
    }
}

// todo from TcpServer:

fn with_handle<F>(addr: SocketAddr, threads: usize, new_service: F)
where
    F: Fn(&Handle) -> LengthProto + Send + Sync + 'static,
{
    let new_service = Arc::new(new_service);
    let workers = threads;

    let threads = (0..threads - 1)
        .map(|i| {
            let new_service = new_service.clone();

            thread::Builder::new()
                .name(format!("worker{}", i))
                .spawn(move || serve(addr, workers, &*new_service))
                .unwrap()
        })
        .collect::<Vec<_>>();

    serve(addr, workers, &*new_service);

    for thread in threads {
        thread.join().unwrap();
    }
}


fn serve<F>(addr: SocketAddr, workers: usize, new_service: F)
where
    F: Fn(&Handle) -> LengthProto,
{
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    //let new_service = new_service(&handle);
    let listener = listener(&addr, workers, &handle).unwrap();

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

fn listener(addr: &SocketAddr, workers: usize, handle: &Handle) -> std::io::Result<TcpListener> {
    let listener = match *addr {
        SocketAddr::V4(_) => net2::TcpBuilder::new_v4()?,
        SocketAddr::V6(_) => net2::TcpBuilder::new_v6()?,
    };
    configure_tcp(workers, &listener)?;
    listener.reuse_address(true)?;
    listener.bind(addr)?;
    listener
        .listen(1024)
        .and_then(|l| TcpListener::from_listener(l, addr, handle))
}

#[cfg(unix)]
fn configure_tcp(workers: usize, tcp: &net2::TcpBuilder) -> std::io::Result<()> {
    use net2::unix::*;

    if workers > 1 {
        (tcp.reuse_port(true))?;
    }

    Ok(())
}

#[cfg(windows)]
fn configure_tcp(_workers: usize, _tcp: &net2::TcpBuilder) -> std::io::Result<()> {
    Ok(())
}
