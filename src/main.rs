use std::{
	net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, sync::Arc
};

use futures::StreamExt;
use quinn::{Certificate, ClientConfig, ClientConfigBuilder, ServerConfigBuilder};
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

	let cert = Endpoint::cert();
	let a = Endpoint::new(None); //Some(cert.clone()));
	let b = Endpoint::new(None); //Some(cert));
	for i in 0..10_000 {
		let receive = async {
			let mut receiver = a.receiver().await;
			let mut buf = [0; 10];
			receiver.read_exact(&mut buf).await.unwrap();
			// println!("{:?}", buf);
		};
		let send = async {
			let mut sender = b.sender(a.pid.clone()).await;
			sender.write_all(b"0123456789").await.unwrap();
			sender.finish().await.unwrap();
		};

		tokio::join!(receive, send);
	}
	tokio::join!(a.wait_idle(), b.wait_idle());

	println!("done");
}

#[derive(Clone)]
struct Pid(SocketAddr, Certificate);

struct Endpoint {
	listen: quinn::Endpoint,
	incoming: quinn::Incoming,
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
	fn new(cert: Option<(quinn::Certificate, quinn::PrivateKey)>) -> Self {
		let (cert, key) = cert.unwrap_or_else(Self::cert);
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
			incoming,
			connect: connect_endpoint,
			pid: Pid(addr, cert),
		}
	}
	async fn sender(&self, pid: Pid) -> quinn::SendStream {
		let mut client_config = ClientConfigBuilder::new(ClientConfig::default());
		client_config.add_certificate_authority(pid.1).unwrap();
		self.connect
			.connect_with(client_config.build(), &pid.0, "a")
			.unwrap()
			.await
			.unwrap()
			.connection
			.open_uni()
			.await
			.unwrap()
	}
	async fn receiver(&self) -> quinn::RecvStream {
		let quinn::NewConnection {
			mut uni_streams, ..
		} = (&self.incoming)
			.next()
			.await
			.expect("accept")
			.await
			.expect("connect");
		uni_streams.next().await.unwrap().unwrap()
	}
	async fn wait_idle(&self) {
		tokio::join!(self.listen.wait_idle(), self.connect.wait_idle());
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
