use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 节点 leaf cert 有效期。续签门槛通常为剩 30 天，故 365 天给两次续签机会。
const NODE_CERT_VALIDITY_DAYS: i64 = 365;

/// 写文件并强制权限。在 Unix 上使用指定模式；其他平台用默认。
fn write_with_mode(path: &str, content: &str, mode: u32) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = opts.open(path).with_context(|| format!("open {path}"))?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

/// 创建目录并设置权限（仅 owner 可访问）
fn create_secure_dir(dir: &str) -> Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).with_context(|| format!("create dir {dir}"))
}

pub struct CertPaths {
    pub ca: String,
    pub ca_key: String,
    pub server: String,
    pub server_key: String,
    pub client: String,
    pub client_key: String,
}

impl CertPaths {
    pub fn under(dir: &str) -> Self {
        let p = |f: &str| format!("{dir}/{f}");
        Self {
            ca: p("ca.pem"),
            ca_key: p("ca-key.pem"),
            server: p("server.pem"),
            server_key: p("server-key.pem"),
            client: p("client.pem"),
            client_key: p("client-key.pem"),
        }
    }

    /// 控制面证书是否齐（不含 CA 私钥；CA 私钥是 master 独有，node 不需要）。
    pub fn ctrl_exists(&self) -> bool {
        [&self.ca, &self.server, &self.server_key, &self.client, &self.client_key]
            .iter()
            .all(|p| Path::new(p).exists())
    }
}

/// 当前 server cert 的 SAN 版本（含 iris-master 身份名后 = v2）。
/// 老版本（仅 localhost + 127.0.0.1）的部署会自动迁移：删 server pair 重签，不动 CA + 共享 client。
const SERVER_CERT_VERSION: &str = "v2-mtls-sni";

/// master 启动时确保证书齐全。CA 一次生成持久化；server pair / 共享 client pair 按需补齐。
/// 旧 server.pem（SAN 仅 localhost+127.0.0.1）会自动迁移到 v2（加 SAN=iris-master），
/// CA + 已签发的 node cert 不动 — 滚动升级安全。
pub fn ensure_dev_certs(dir: &str) -> Result<CertPaths> {
    let paths = CertPaths::under(dir);
    create_secure_dir(dir).ok();

    // CA：缺则生成 + 持久化；存在则加载（共享 ca_cert 用于签 leaf）
    let (ca_cert, ca_key) = if Path::new(&paths.ca).exists() && Path::new(&paths.ca_key).exists() {
        load_ca(dir)?
    } else {
        let ca_key = KeyPair::generate().context("gen ca key")?;
        let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.distinguished_name.push(DnType::CommonName, "iris-ca");
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).context("self-sign ca")?;
        fs::write(&paths.ca, ca_cert.pem())?;
        write_with_mode(&paths.ca_key, &ca_key.serialize_pem(), 0o600)?;
        (ca_cert, ca_key)
    };

    // server pair：用版本标记决定要不要重签。缺标记 = 旧版（仅 localhost SAN）→ 自动迁移。
    let version_marker = format!("{dir}/.server-cert-version");
    let server_missing =
        !Path::new(&paths.server).exists() || !Path::new(&paths.server_key).exists();
    let version_stale = !matches!(
        fs::read_to_string(&version_marker),
        Ok(v) if v.trim() == SERVER_CERT_VERSION
    );
    if server_missing || version_stale {
        // SAN 加 iris-master（节点 dial 时 SNI 用此身份名）；保留 localhost+127.0.0.1 兼容旧 client。
        let san = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "iris-master".to_string(),
        ];
        let (server_pem, server_key_pem) = leaf(&san, "iris-master", &ca_cert, &ca_key)?;
        fs::write(&paths.server, server_pem)?;
        write_with_mode(&paths.server_key, &server_key_pem, 0o600)?;
        write_with_mode(&version_marker, SERVER_CERT_VERSION, 0o600)?;
    }

    // 共享 client pair：master 反向 probe 节点时复用（per-call domain_name=目标 node_id）。缺则补。
    if !Path::new(&paths.client).exists() || !Path::new(&paths.client_key).exists() {
        let san = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let (client_pem, client_key_pem) = leaf(&san, "iris-node", &ca_cert, &ca_key)?;
        fs::write(&paths.client, client_pem)?;
        write_with_mode(&paths.client_key, &client_key_pem, 0o600)?;
    }

    Ok(paths)
}

/// 加载已持久化的 CA（cert + key），用于签发节点专属证书。
pub fn load_ca(dir: &str) -> Result<(rcgen::Certificate, KeyPair)> {
    let paths = CertPaths::under(dir);
    let key_pem = fs::read_to_string(&paths.ca_key)
        .with_context(|| format!("read ca-key {}", paths.ca_key))?;
    let cert_pem = fs::read_to_string(&paths.ca)
        .with_context(|| format!("read ca {}", paths.ca))?;
    let key = KeyPair::from_pem(&key_pem).context("parse ca key")?;
    // 重新解析 CA 证书参数 + 重签得到可用于签发的 Certificate 句柄
    let params = CertificateParams::from_ca_cert_pem(&cert_pem).context("parse ca cert params")?;
    let cert = params.self_signed(&key).context("rebuild ca cert")?;
    Ok((cert, key))
}

/// 为指定节点签发独立 mTLS 客户端证书（CN=node_id），由 master CA 签发。
pub fn sign_node_cert(
    dir: &str,
    node_id: &str,
) -> Result<(String /* cert pem */, String /* key pem */, String /* ca pem */)> {
    let (ca_cert, ca_key) = load_ca(dir)?;
    let key = KeyPair::generate().context("gen node key")?;
    let san = vec!["localhost".to_string(), node_id.to_string()];
    let mut params = CertificateParams::new(san)?;
    params
        .distinguished_name
        .push(DnType::CommonName, format!("iris-node-{node_id}"));
    // 显式设有效期：续签流程依赖 not_after 可读 + 在 30 天阈值内主动调 RenewCert。
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(NODE_CERT_VALIDITY_DAYS);
    let cert = params.signed_by(&key, &ca_cert, &ca_key).context("sign node leaf")?;
    let ca_pem = fs::read_to_string(format!("{dir}/ca.pem"))?;
    Ok((cert.pem(), key.serialize_pem(), ca_pem))
}

/// 解析 PEM 编码的 X.509 证书，返回 NotAfter 毫秒时间戳（UNIX epoch）。
/// 节点启动时读取自身 cert，每次 heartbeat 上报供 UI 显示倒计时；
/// renew task 用此判断是否进入 30 天续签窗口。
pub fn cert_not_after_ms(pem: &[u8]) -> Result<i64> {
    use x509_parser::pem::parse_x509_pem;
    let (_, p) = parse_x509_pem(pem).map_err(|e| anyhow::anyhow!("parse pem: {e}"))?;
    let cert = p.parse_x509().context("parse x509")?;
    let secs = cert.validity().not_after.timestamp();
    Ok(secs.saturating_mul(1000))
}

/// 现在毫秒时间戳，供节点端 renew task 比较。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 编译时注入的 git short hash（8 字符）— 过滤版，仅 node/common/proto 变化才滚。
/// 节点版本上报使用此值，避免 UI/master-only 改动让所有节点显示 outdated。
/// `unknown` 表示非 git 仓库 / shallow clone。
pub const GIT_HASH: &str = env!("IRIS_GIT_HASH");

/// 真 HEAD short hash（每次提交必滚）。master 自更新轮询用此 hash 与 GitHub HEAD 比对,
/// 区别于 GIT_HASH（被 node 过滤逻辑用），两者语义独立。
pub const MASTER_HEAD_HASH: &str = env!("IRIS_MASTER_HEAD_HASH");

/// Cargo.toml workspace 版本号（如 "0.1.0"）。
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 编译时 UNIX 秒时间戳，区分同 git hash 的多次构建。
pub const BUILD_TS: &str = env!("IRIS_BUILD_TS");

/// 完整版本字符串："0.1.0-d28db4e1"。用于上报 + UI 显示。
pub fn version_string() -> String {
    format!("{PKG_VERSION}-{GIT_HASH}")
}

fn leaf(
    san: &[String],
    cn: &str,
    ca_cert: &rcgen::Certificate,
    ca_key: &KeyPair,
) -> Result<(String, String)> {
    let key = KeyPair::generate().context("gen leaf key")?;
    let mut params = CertificateParams::new(san.to_vec())?;
    params.distinguished_name.push(DnType::CommonName, cn);
    let cert = params.signed_by(&key, ca_cert, ca_key).context("sign leaf")?;
    Ok((cert.pem(), key.serialize_pem()))
}
