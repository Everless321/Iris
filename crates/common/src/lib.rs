use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use std::fs;
use std::io::Write;
use std::path::Path;

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

/// master 启动时确保证书齐全；首次启动生成 CA + server + 一份共享 client（保留供 dev 使用）。
/// CA 私钥（ca-key.pem）会持久化，用于后续按需签发节点专属证书。
pub fn ensure_dev_certs(dir: &str) -> Result<CertPaths> {
    let paths = CertPaths::under(dir);
    if paths.ctrl_exists() && Path::new(&paths.ca_key).exists() {
        return Ok(paths);
    }
    create_secure_dir(dir)?;

    // CA
    let ca_key = KeyPair::generate().context("gen ca key")?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(DnType::CommonName, "zhuanfa-ca");
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca_params.self_signed(&ca_key).context("self-sign ca")?;

    let san = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let (server_pem, server_key_pem) = leaf(&san, "zhuanfa-master", &ca_cert, &ca_key)?;
    let (client_pem, client_key_pem) = leaf(&san, "zhuanfa-node", &ca_cert, &ca_key)?;

    // 证书(public)用默认 644；私钥严格 600（CA 私钥泄露 → 整个信任链失守）
    fs::write(&paths.ca, ca_cert.pem())?;
    write_with_mode(&paths.ca_key, &ca_key.serialize_pem(), 0o600)?;
    fs::write(&paths.server, server_pem)?;
    write_with_mode(&paths.server_key, &server_key_pem, 0o600)?;
    fs::write(&paths.client, client_pem)?;
    write_with_mode(&paths.client_key, &client_key_pem, 0o600)?;
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
        .push(DnType::CommonName, format!("zhuanfa-node-{node_id}"));
    let cert = params.signed_by(&key, &ca_cert, &ca_key).context("sign node leaf")?;
    let ca_pem = fs::read_to_string(format!("{dir}/ca.pem"))?;
    Ok((cert.pem(), key.serialize_pem(), ca_pem))
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
