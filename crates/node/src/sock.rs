use std::io;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpSocket, TcpStream, UdpSocket};

/// UDP socket 缓冲区（4 MB）。仅用于 UDP；TCP 走 Linux autotune。
/// 部署侧 sysctl 推荐：`net.core.rmem_max=4194304 net.core.wmem_max=4194304`
pub const SOCK_BUF: u32 = 4 * 1024 * 1024;

/// 不动 SO_SNDBUF / SO_RCVBUF —— 让 Linux TCP autotune 工作。
/// 早期代码硬设 4MB 会触发：
///   1) 关闭 autotune（SOCK_RCVBUF_LOCK / SOCK_SNDBUF_LOCK）
///   2) connect/listen 前 rcv_wscale 按当前 rcvbuf 计算 → 实测仅 2
///   3) advertised window 卡在 256KB → 单流 BDP cap 在 5 Gbps（10G NIC）
/// realm / shadowsocks-rust / pingora 都采用 autotune，对齐之。
fn tcp_socket_for(sa: &SocketAddr) -> io::Result<TcpSocket> {
    if sa.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
}

/// 调好缓冲区 + SO_REUSEADDR 的 TCP 监听
pub fn tcp_listen(addr: SocketAddr) -> io::Result<TcpListener> {
    let s = tcp_socket_for(&addr)?;
    s.set_reuseaddr(true)?;
    s.bind(addr)?;
    s.listen(1024)
}

/// 调好缓冲区 + TCP_NODELAY 的 TCP 拨号；接受 host:port 也接受 ip:port
pub async fn tcp_connect(addr: &str) -> io::Result<TcpStream> {
    let mut last: Option<io::Error> = None;
    for sa in tokio::net::lookup_host(addr).await? {
        let s = match tcp_socket_for(&sa) {
            Ok(x) => x,
            Err(e) => {
                last = Some(e);
                continue;
            }
        };
        match s.connect(sa).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "no address resolved")))
}

/// accept 出来的 inbound 也要 NODELAY，避免 Nagle 攒包
pub fn tune_accepted(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
}

/// 调好缓冲区的 UDP bind（tokio UdpSocket 没暴露 buf API，走 socket2）
pub fn udp_bind(addr: SocketAddr) -> io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let s = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    let _ = s.set_recv_buffer_size(SOCK_BUF as usize);
    let _ = s.set_send_buffer_size(SOCK_BUF as usize);
    s.set_reuse_address(true)?;
    s.set_nonblocking(true)?;
    s.bind(&SockAddr::from(addr))?;
    let std_sock: std::net::UdpSocket = s.into();
    UdpSocket::from_std(std_sock)
}
