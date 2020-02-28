use futures::StreamExt;
use quinn::{
	Certificate, ClientConfig, ClientConfigBuilder, ConnectionError, ServerConfigBuilder
};
use quinn_proto::TransportErrorCode;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{
	net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, sync::Arc, time::Duration
};
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
	let mut rng = SmallRng::seed_from_u64(0);

	let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
	let key = quinn::PrivateKey::from_der(&cert.serialize_private_key_der()).unwrap();
	let cert = Certificate::from_der(&cert.serialize_der().unwrap()).unwrap();
	let cert_chain = quinn::CertificateChain::from_certs(vec![cert.clone()]);

	let mut transport = quinn::TransportConfig::default();
	transport.idle_timeout(None).unwrap();
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
	let (server_endpoint, mut incoming) = server_endpoint.with_socket(sock).unwrap();

	let (client_endpoint, _) = quinn::Endpoint::builder()
		.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
		.unwrap();

	let mut client_config = ClientConfig::default();
	client_config.transport = transport.clone();
	let mut client_config = ClientConfigBuilder::new(client_config);
	client_config.add_certificate_authority(cert.clone()).unwrap();
	let client_config = client_config.build();

	let iterations = 100_000_000;
	let mut shared_rng = rng.clone();
	let shared_rng1 = shared_rng.clone();
	let rng1 = SmallRng::from_rng(&mut rng).unwrap();
	let client_endpoint = &client_endpoint;
	let sending = async move {
		let mut shared_rng = shared_rng1;
		let mut rng = rng1;
		let mut sender = None;
		for i in 0..iterations {
			if sender.is_none() {
				println!("{} sender open", i);
				let connection = loop {
					match client_endpoint
						.connect_with(client_config.clone(), &addr, "localhost")
						.unwrap()
						.await
					{
						Ok(connection) => break connection,
						Err(ConnectionError::ConnectionClosed(close))
							if close.error_code == TransportErrorCode::SERVER_BUSY =>
						{
							delay_for(Duration::from_millis(10)).await
						}
						Err(err) => panic!("{:?}", err),
					}
				};
				let mut sender_ = connection.connection.open_uni().await.unwrap();
				sender_.write_all(&[1, 2]).await.unwrap();
				sender = Some(sender_);
				println!("{} sender /open", i);
			} else if shared_rng.gen() {
				println!("{} sender close", i);
				match sender.take().unwrap().finish().await {
					Ok(())
					| Err(quinn::WriteError::ConnectionClosed(
						quinn::ConnectionError::ApplicationClosed(_),
					)) => (),
					Err(err) => panic!("{:?}", err),
				}
				println!("{} sender /close", i);
			} else {
				println!("{} sender send", i);
				sender.as_mut().unwrap().write_all(b"0123456789").await.unwrap();
				println!("{} sender /send", i);
			}
			if rng.gen() {
				delay_for(rng.gen_range(Duration::new(0, 0), Duration::from_micros(500))).await;
			}
		}
		if let Some(mut sender) = sender {
			sender.finish().await.unwrap();
		}
	};
	let receiving = async move {
		let mut receiver = None;
		for i in 0..iterations {
			if receiver.is_none() {
				println!("{} receiver open", i);
				let connecting = incoming.next().await;
				let quinn::NewConnection { mut uni_streams, .. } =
					connecting.expect("accept").await.expect("connect");
				let mut receiver_ = uni_streams.next().await.unwrap().unwrap();
				let mut buf = [0; 2];
				receiver_.read_exact(&mut buf).await.unwrap();
				assert_eq!(buf, [1, 2]);
				receiver = Some(receiver_);
				println!("{} receiver /open", i);
			} else if shared_rng.gen() {
				println!("{} receiver close", i);
				// let fin = receiver.as_mut().unwrap().read(&mut [0]).await.unwrap();
				// assert!(fin.is_none());
				drop(receiver.take().unwrap());
				println!("{} receiver /close", i);
			} else {
				println!("{} receiver recv", i);
				let mut buf = [0; 10];
				receiver.as_mut().unwrap().read_exact(&mut buf).await.unwrap();
				assert_eq!(&buf, b"0123456789");
				println!("{} receiver /recv", i);
			}
			if rng.gen() {
				delay_for(rng.gen_range(Duration::new(0, 0), Duration::from_micros(500))).await;
			}
		}
	};
	tokio::join!(sending, receiving);
	tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());

	println!("done");
}
