use futures::{future::select_all, lock::Mutex, stream::FuturesUnordered, FutureExt, StreamExt};
use quinn::{
	Certificate, ClientConfig, ClientConfigBuilder, ConnectionError, NewConnection, RecvStream, ServerConfigBuilder, WriteError
};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{
	future::Future, net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, pin::Pin, sync::Arc
};

#[tokio::main]
async fn main() {
	tracing_subscriber::fmt::init();

	let rng = SmallRng::seed_from_u64(0);

	let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
	let key = quinn::PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
	let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();
	let cert_chain = quinn::CertificateChain::from_certs(vec![cert.clone()]);

	let mut transport = quinn::TransportConfig::default();
	transport.max_idle_timeout(None).unwrap();
	let transport = Arc::new(transport);

	let mut server_config = quinn::ServerConfig::default();
	server_config.transport = transport.clone();
	let mut server_config = ServerConfigBuilder::new(server_config);
	server_config.certificate(cert_chain, key).unwrap();
	let server_config = server_config.build();

	let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)).unwrap();
	let addr = sock.local_addr().unwrap();

	let mut server_endpoint = quinn::Endpoint::builder();
	server_endpoint.listen(server_config);
	let (server_endpoint, incoming) = server_endpoint.with_socket(sock).unwrap();
	let incoming = Mutex::new(incoming);

	let (client_endpoint, _) = quinn::Endpoint::builder()
		.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
		.unwrap();

	let mut client_config = ClientConfig::default();
	client_config.transport = transport.clone();
	let mut client_config = ClientConfigBuilder::new(client_config);
	client_config.add_certificate_authority(cert.clone()).unwrap();
	let client_config = client_config.build();

	let iterations = 100_000;
	let mut shared_rng = rng.clone();
	let mut shared_rng1 = shared_rng.clone();
	let mut sender = None;
	let mut receiver = None;
	for i in 0..iterations {
		if i % 100 == 0 {
			println!("{}", i);
		}
		let sending = async {
			let shared_rng = &mut shared_rng1;
			if sender.is_none() {
				let connection = client_endpoint
					.connect_with(client_config.clone(), &addr, "localhost")
					.unwrap();
				let connection = connection.await.unwrap();
				let mut sender_ = connection.connection.open_uni().await.unwrap();
				sender_.write_all(&[1]).await.unwrap();
				sender = Some(sender_);
			} else if shared_rng.gen() {
				match sender.take().unwrap().finish().await {
					Ok(())
					| Err(WriteError::ConnectionClosed(ConnectionError::ApplicationClosed(_)))
					| Err(WriteError::ConnectionClosed(ConnectionError::Reset)) => (),
					Err(err) => panic!("{:?}", err),
				}
			} else {
				sender.as_mut().unwrap().write_all(b"0123456789").await.unwrap();
			}
		};
		let receiving = async {
			if receiver.is_none() {
				// let pool = Arc::new(FuturesUnordered::new());
				// fn accept<'a>(
				// 	pool: Arc<FuturesUnordered<Pin<Box<dyn Future<Output = RecvStream> + 'a>>>>,
				// 	incoming: &'a Mutex<quinn::Incoming>, i: usize,
				// ) -> impl Future<Output = RecvStream> + 'a {
				// 	async move {
				// 		let connecting = incoming.lock().await.next().await.expect("accept");
				// 		pool.push(Box::pin(accept(pool.clone(), incoming, i)));
				// 		let NewConnection { mut uni_streams, .. } =
				// 			connecting.await.expect("connect");
				// 		let mut receiver = uni_streams.next().await.unwrap().unwrap();
				// 		let mut buf = [0; 1];
				// 		receiver.read_exact(&mut buf).await.unwrap();
				// 		assert_eq!(buf, [1]);
				// 		receiver
				// 	}
				// }
				// pool.push(Box::pin(accept(pool.clone(), &incoming, i)));
				// receiver = Some(pool.next().await.unwrap());
				receiver = Some(
					select_all((0..10).map(|_| {
						async {
							let connecting = incoming.lock().await.next().await.expect("accept");
							let NewConnection { mut uni_streams, .. } =
								connecting.await.expect("connect");
							let mut receiver = uni_streams.next().await.unwrap().unwrap();
							let mut buf = [0; 1];
							receiver.read_exact(&mut buf).await.unwrap();
							assert_eq!(buf, [1]);
							receiver
						}
						.boxed_local()
					}))
					.await
					.0,
				);
			} else if shared_rng.gen() {
				let fin = receiver.take().unwrap().read(&mut [0]).await.unwrap();
				assert!(fin.is_none());
			} else {
				let mut buf = [0; 10];
				receiver.as_mut().unwrap().read_exact(&mut buf).await.unwrap();
				assert_eq!(&buf, b"0123456789");
			}
		};
		tokio::join!(sending, receiving);
	}
	if let Some(mut sender) = sender {
		sender.finish().await.unwrap();
	}
	if let Some(mut receiver) = receiver {
		let fin = receiver.read(&mut [0]).await.unwrap();
		assert!(fin.is_none());
	}
	tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());
}
