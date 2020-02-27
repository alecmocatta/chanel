use futures::StreamExt;
use quinn::{Certificate, ClientConfig, ClientConfigBuilder, ServerConfigBuilder};
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

	let transport = quinn::TransportConfig::default();
	let mut server_config = quinn::ServerConfig::default();
	server_config.transport = Arc::new(transport);
	let mut server_config = ServerConfigBuilder::new(server_config);
	server_config.certificate(cert_chain, key).unwrap();

	let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)).unwrap();
	let addr = sock.local_addr().unwrap();

	let mut listen_endpoint = quinn::Endpoint::builder();
	listen_endpoint.listen(server_config.build());
	let (listen_endpoint, mut incoming) = listen_endpoint.with_socket(sock).unwrap();

	let (connect_endpoint, _) = quinn::Endpoint::builder()
		.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))
		.unwrap();

	let iterations = 100_000;
	let mut shared_rng = rng.clone();
	let shared_rng1 = shared_rng.clone();
	let rng1 = SmallRng::from_rng(&mut rng).unwrap();
	let connect_endpoint = &connect_endpoint;
	let sending = async move {
		let mut shared_rng = shared_rng1;
		let mut rng = rng1;
		let mut sender = None;
		for i in 0..iterations {
			if sender.is_none() {
				println!("{} sender open", i);
				let mut client_config = ClientConfigBuilder::new(ClientConfig::default());
				client_config.add_certificate_authority(cert.clone()).unwrap();
				let connection = connect_endpoint
					.connect_with(client_config.build(), &addr, "localhost")
					.unwrap()
					.await
					.unwrap();
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
				delay_for(rng.gen_range(Duration::new(0, 0), Duration::from_millis(1))).await;
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
				delay_for(rng.gen_range(Duration::new(0, 0), Duration::from_millis(1))).await;
			}
		}
	};
	tokio::join!(sending, receiving);
	tokio::join!(connect_endpoint.wait_idle(), listen_endpoint.wait_idle());

	println!("done");
}
