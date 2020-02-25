use std::{
	net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, sync::{Arc, Mutex}
};

use futures::{
	future::{join_all, Either}, StreamExt
};
use itertools::Itertools;
use quinn::{Certificate, ClientConfig, ClientConfigBuilder, ServerConfigBuilder};
use rand::{rngs::SmallRng, Rng, SeedableRng};
// use tokio::runtime::{Builder, Runtime};
// use tokio::prelude::*;

#[tokio::main]
async fn main() {
	// tracing::subscriber::set_global_default(
	//     tracing_subscriber::FmtSubscriber::builder()
	//         .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
	//         .finish(),
	// )
	// .unwrap();

	let rng = SmallRng::seed_from_u64(0);
	let endpoints = 2;
	let iterations = 100_000;
	println!(
		"{} channels: {:?}",
		(0..endpoints).permutations(2).count(),
		(0..endpoints).permutations(2).collect::<Vec<_>>()
	);
	let endpoints = (0..endpoints).map(|_| Endpoint::new()).collect::<Vec<_>>();
	join_all(endpoints.iter().permutations(2).flat_map(|ends| {
		let sender_endpoint = ends[0];
		let receiver_endpoint = ends[1];
		let rng = SmallRng::from_rng(rng.clone()).unwrap();
		let rng1 = rng.clone();
		let sending = async move {
			println!("sending");
			let mut rng = rng1;
			let mut sender = None;
			for i in 0..iterations {
				if i % 1000 == 0 {
					println!("s {}", i)
				};
				if rng.gen() {
					if sender.is_none() {
						// println!("s create");
						sender = Some(sender_endpoint.sender(receiver_endpoint.pid()).await);
					// println!("s /create");
					} else {
						// println!("s drop");
						sender.take().unwrap().finish().await.unwrap();
						// println!("s /drop");
					}
				} else {
					if let Some(sender) = &mut sender {
						// println!("s write");
						sender.write_all(b"0123456789").await.unwrap();
						// println!("s /write");
					}
				}
			}
			if let Some(mut sender) = sender {
				// println!("s finish");
				sender.finish().await.unwrap();
				// println!("s /finish");
			}
		};
		let receiving = async move {
			println!("receiving");
			let mut rng = rng;
			let mut receiver = None;
			for i in 0..iterations {
				if i % 1000 == 0 {
					println!("r {}", i)
				};
				if rng.gen() {
					if receiver.is_none() {
						// println!("r create");
						receiver = Some(receiver_endpoint.receiver(sender_endpoint.pid()).await);
					// println!("r /create");
					} else {
						// println!("r drop");
						receiver.take().unwrap(); //.finish().await.unwrap();
						  // println!("r /drop");
					}
				} else {
					if let Some(receiver) = &mut receiver {
						let mut buf = [0; 10];
						// println!("r read");
						receiver.read_exact(&mut buf).await.unwrap();
						// println!("r /read");
					}
				}
			}
			// if let Some(receiver) = receiver {
			// receiver.finish().await.unwrap();
			// }
		};
		println!("both");
		vec![Either::Left(sending), Either::Right(receiving)]
	}))
	.await;
	join_all(endpoints.iter().map(|endpoint| endpoint.wait_idle())).await;

	// 	let receiver = receiver_endpoint.receiver();
	// for i in 0..10_000 {
	// 	if i % 100 == 0 { println!("{}", i) }
	// 	if rng.gen() {
	// 		if rng.gen() {
	// 			endpoints.push(Endpoint::new(None));
	// 		// } else {
	// 			// delete en
	// 		}
	// 	}
	// }
	// let a = Endpoint::new(None); //Some(cert.clone()));
	// let b = Endpoint::new(None); //Some(cert));
	// {
	// 	let receive = async {
	// 		let mut receiver = a.receiver().await;
	// 		let mut buf = [0; 10];
	// 		receiver.read_exact(&mut buf).await.unwrap();
	// 		// println!("{:?}", buf);
	// 	};
	// 	let send = async {
	// 		let mut sender = b.sender(a.pid.clone()).await;
	// 		sender.write_all(b"0123456789").await.unwrap();
	// 		sender.finish().await.unwrap();
	// 	};

	// 	tokio::join!(receive, send);
	// }
	// tokio::join!(a.wait_idle(), b.wait_idle());

	println!("done");
}

#[derive(Clone)]
pub struct Pid(SocketAddr, Certificate);

pub struct Endpoint {
	listen: quinn::Endpoint,
	incoming: Mutex<quinn::Incoming>,
	connect: quinn::Endpoint,
	pid: Pid,
}
impl Endpoint {
	fn cert() -> (quinn::Certificate, quinn::PrivateKey) {
		let cert = rcgen::generate_simple_self_signed(vec!["a".into()]).unwrap();
		let key = quinn::PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
		let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();
		(cert, key)
	}
	pub fn new() -> Self {
		let (cert, key) = Self::cert();
		let cert_chain = quinn::CertificateChain::from_certs(vec![cert.clone()]);

		let transport = quinn::TransportConfig::default();
		let mut server_config = quinn::ServerConfig::default();
		server_config.transport = Arc::new(transport);
		let mut server_config = ServerConfigBuilder::new(server_config);
		server_config.certificate(cert_chain, key).unwrap();

		let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)).unwrap();
		let addr = sock.local_addr().unwrap();

		let mut listen_endpoint = quinn::Endpoint::builder();
		listen_endpoint.listen(server_config.build());
		let (listen_endpoint, incoming) = listen_endpoint.with_socket(sock).unwrap();

		let (connect_endpoint, _) = quinn::Endpoint::builder()
			.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
			.unwrap();

		Self {
			listen: listen_endpoint,
			incoming: Mutex::new(incoming),
			connect: connect_endpoint,
			pid: Pid(addr, cert),
		}
	}
	pub async fn sender(&self, pid: Pid) -> quinn::SendStream {
		let mut client_config = ClientConfigBuilder::new(ClientConfig::default());
		client_config.add_certificate_authority(pid.1).unwrap();
		let mut sender = self
			.connect
			.connect_with(client_config.build(), &pid.0, "a")
			.unwrap()
			.await
			.unwrap()
			.connection
			.open_uni()
			.await
			.unwrap();
		sender
			.write_all(&self.pid.0.port().to_ne_bytes())
			.await
			.unwrap();
		sender
	}
	pub async fn receiver(&self, pid: Pid) -> quinn::RecvStream {
		let quinn::NewConnection {
			mut uni_streams, ..
		} = self
			.incoming
			.lock()
			.unwrap()
			.next()
			.await
			.expect("accept")
			.await
			.expect("connect");
		let mut receiver = uni_streams.next().await.unwrap().unwrap();
		let mut buf = [0; 2];
		receiver.read_exact(&mut buf).await.unwrap();
		let port = u16::from_ne_bytes(buf);
		assert_eq!(port, pid.0.port(), "todo");
		receiver
	}
	pub async fn wait_idle(&self) {
		tokio::join!(self.listen.wait_idle(), self.connect.wait_idle());
	}
	pub fn pid(&self) -> Pid {
		self.pid.clone()
	}
}

// pub async fn spawn_server() -> (SocketAddr, Certificate, tokio::task::JoinHandle<()>) {
// 	let cert = rcgen::generate_simple_self_signed(vec!["a".into()]).unwrap();
// 	let key = quinn::PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
// 	let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();
// 	let cert_chain = quinn::CertificateChain::from_certs(vec![cert.clone()]);

// 	let mut transport = quinn::TransportConfig::default();
// 	transport.stream_window_uni(1024);
// 	let mut server_config = quinn::ServerConfig::default();
// 	server_config.transport = Arc::new(transport);
// 	let mut server_config = ServerConfigBuilder::new(server_config);
// 	server_config.certificate(cert_chain, key).unwrap();

// 	let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)).unwrap();
// 	let addr = sock.local_addr().unwrap();
// 	let config = server_config.build();
// 	let handle = tokio::spawn({
// 		let mut endpoint = quinn::Endpoint::builder();
// 		endpoint.listen(config);
// 		// let mut runtime = rt();
// 		// let (_, mut incoming) = runtime.enter(|| );
// 		//runtime.spawn(
// 		async move {
// 			let (_, mut incoming) = endpoint.with_socket(sock).unwrap();
// 			let quinn::NewConnection {
// 				mut uni_streams, ..
// 			} = incoming
// 				.next()
// 				.await
// 				.expect("accept")
// 				.await
// 				.expect("connect");
// 			while let Some(Ok(mut stream)) = uni_streams.next().await {
// 				while let Some(_) = stream.read_unordered().await.unwrap() {}
// 			}
// 		}
// 		// .instrument(error_span!("server")),
// 		// runtime.block_on(handle);
// 	});
// 	(addr, cert, handle)
// }

// pub async fn make_client(
// 	server_addr: SocketAddr, cert: Certificate,
// ) -> (quinn::Endpoint, quinn::Connection) {
// 	let mut client_config = ClientConfigBuilder::new(ClientConfig::default());
// 	client_config.add_certificate_authority(cert).unwrap();

// 	// let mut runtime = rt();
// 	// let (endpoint, _) = runtime.enter(|| {
// 	//     Endpoint::builder()
// 	//         .bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
// 	//         .unwrap()
// 	// });
// 	// let (endpoint, quinn::NewConnection { connection, .. }) = runtime.block_on(async {
// 	let (endpoint, _) = quinn::Endpoint::builder()
// 		.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
// 		.unwrap();
// 	let ret = endpoint
// 		.connect_with(client_config.build(), &server_addr, "a")
// 		.unwrap()
// 		.await
// 		.unwrap();
// 	(endpoint, ret.connection)
// 	// .instrument(error_span!("client"))
// 	// });
// 	// (endpoint, connection, runtime)
// }

// fn rt() -> Runtime {
// 	Builder::new()
// 		.basic_scheduler()
// 		.enable_all()
// 		.build()
// 		.unwrap()
// }
