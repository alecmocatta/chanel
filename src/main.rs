use futures::{
	future::{join_all, Either}, StreamExt
};
use itertools::Itertools;
use quinn::{Certificate, ClientConfig, ClientConfigBuilder, ServerConfigBuilder};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{
	net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, sync::Arc
};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
	let rng = SmallRng::seed_from_u64(0);
	let endpoints = 2;
	let iterations = 100_000;
	println!(
		"{} channels: {:?}",
		(0..endpoints).permutations(2).count(),
		(0..endpoints).permutations(2).collect::<Vec<_>>()
	);
	let endpoints = (0..endpoints).map(|_| Endpoint::new()).collect::<Vec<_>>();
	join_all(endpoints.iter().permutations(2).enumerate().flat_map(|(i, ends)| {
		let sender_endpoint = ends[0];
		let receiver_endpoint = ends[1];
		let rng = SmallRng::from_rng(rng.clone()).unwrap();
		let rng1 = rng.clone();
		let sending = async move {
			println!("sending {}", i);
			let mut rng = rng1;
			let mut sender = None;
			for i in 0..iterations {
				if i % 1000 == 0 {
					println!("s {}", i)
				};
				if rng.gen() {
					if sender.is_none() {
						sender = Some(sender_endpoint.sender(receiver_endpoint.pid()).await);
					} else {
						sender.take().unwrap().finish().await.unwrap();
					}
				} else {
					if let Some(sender) = &mut sender {
						sender.write_all(b"0123456789").await.unwrap();
					}
				}
			}
			if let Some(mut sender) = sender {
				sender.finish().await.unwrap();
			}
		};
		let receiving = async move {
			println!("receiving {}", i);
			let mut rng = rng;
			let mut receiver = None;
			for i in 0..iterations {
				if i % 1000 == 0 {
					println!("r {}", i)
				};
				if rng.gen() {
					if receiver.is_none() {
						receiver = Some(receiver_endpoint.receiver(sender_endpoint.pid()).await);
					} else {
						receiver.take().unwrap();
					}
				} else {
					if let Some(receiver) = &mut receiver {
						let mut buf = [0; 10];
						receiver.read_exact(&mut buf).await.unwrap();
					}
				}
			}
		};
		println!("channel {}", i);
		vec![Either::Left(sending), Either::Right(receiving)]
	}))
	.await;
	join_all(endpoints.iter().map(|endpoint| endpoint.wait_idle())).await;

	println!("done");
}

#[derive(Clone)]
pub struct Pid(SocketAddr, Certificate);

const CERT_DOMAIN: &str = "a";

pub struct Endpoint {
	listen: quinn::Endpoint,
	incoming: Mutex<quinn::Incoming>,
	connect: quinn::Endpoint,
	pid: Pid,
}
impl Endpoint {
	fn cert() -> (quinn::Certificate, quinn::PrivateKey) {
		let cert = rcgen::generate_simple_self_signed(vec![CERT_DOMAIN.into()]).unwrap();
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
		let connection = self
			.connect
			.connect_with(client_config.build(), &pid.0, CERT_DOMAIN)
			.unwrap()
			.await
			.unwrap();
		let mut sender = connection.connection.open_uni().await.unwrap();
		sender.write_all(&self.pid.0.port().to_ne_bytes()).await.unwrap();
		sender
	}
	pub async fn receiver(&self, pid: Pid) -> quinn::RecvStream {
		let connecting = self.incoming.lock().await.next().await;
		let quinn::NewConnection { mut uni_streams, .. } =
			connecting.expect("accept").await.expect("connect");
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
