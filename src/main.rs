use futures::StreamExt;
use quinn::{Certificate, ClientConfig, ClientConfigBuilder, NewConnection, ServerConfigBuilder};
use std::{
	convert::TryFrom, net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket}, sync::Arc
};

#[tokio::main]
async fn main() {
	tracing_subscriber::fmt::init();

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

	let connection =
		client_endpoint.connect_with(client_config.clone(), &addr, "localhost").unwrap();
	let connection = connection.await.unwrap().connection;

	let connecting = incoming.next().await.expect("accept");
	let NewConnection { mut uni_streams, .. } = connecting.await.expect("connect");

	let mut sender = connection.open_uni().await.unwrap();
	sender.write_all(&[1]).await.unwrap();

	let mut receiver = uni_streams.next().await.unwrap().unwrap();
	let mut buf = [0; 1];
	receiver.read_exact(&mut buf).await.unwrap();
	assert_eq!(buf, [1]);

	let iterations = 100_000_000;
	const BUF: usize = 100 * 1024;
	let mut send_buf = vec![0; BUF];
	for (i, chunk) in send_buf.chunks_mut(4).enumerate() {
		*<&mut [u8; 4]>::try_from(chunk).unwrap() = u32::try_from(i).unwrap().to_be_bytes();
	}
	let mut recv_buf = vec![0; BUF];
	let start = std::time::Instant::now();
	for i in 0..iterations {
		let sending = async {
			sender.write_all(&send_buf).await.unwrap();
		};
		let receiving = async {
			receiver.read_exact(&mut recv_buf).await.unwrap();
			if send_buf != recv_buf {
				panic!(
					"{:?} {}:\nleft:\n{:02x?}\nright:\n{:02x?}",
					start.elapsed(),
					i,
					&send_buf,
					&recv_buf,
				);
			}
			recv_buf.iter_mut().for_each(|x| *x = 0);
		};
		tokio::join!(sending, receiving);
	}
	drop(connection);
	drop(uni_streams);
	sender.finish().await.unwrap();
	let fin = receiver.read(&mut [0]).await.unwrap();
	assert!(fin.is_none());
	tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());

	println!("done");
}
