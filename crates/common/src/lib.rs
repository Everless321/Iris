use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use std::fs;
use std::path::Path;

/// mTLS 证书材料的磁盘布局。
pub struct CertPaths {
    pub ca: String,
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
            server: p("server.pem"),
            server_key: p("server-key.pem"),
            client: p("client.pem"),
            client_key: p("client-key.pem"),
        }
    }

    pub fn all_exist(&self) -> bool {
        [&self.ca, &self.server, &self.server_key, &self.client, &self.client_key]
            .iter()
            .all(|p| Path::new(p).exists())
    }
}

/// 若证书目录不完整，则生成全套 mTLS 材料：
/// 内置 CA → 签发 server 证书（SAN: localhost / 127.0.0.1）+ client 证书。
/// P0 同机验证用；P2 起改为 master 持有 CA、按 enrollment 动态签发节点证书。
pub fn ensure_dev_certs(dir: &str) -> Result<CertPaths> {
    let paths = CertPaths::under(dir);
    if paths.all_exist() {
        return Ok(paths);
    }
    fs::create_dir_all(dir).with_context(|| format!("create cert dir {dir}"))?;

    // CA
    let ca_key = KeyPair::generate().context("gen ca key")?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(DnType::CommonName, "zhuanfa-dev-ca");
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca_params.self_signed(&ca_key).context("self-sign ca")?;

    // server / client 叶证书，均由 CA 签发
    let san = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let (server_pem, server_key_pem) = leaf(&san, "zhuanfa-master", &ca_cert, &ca_key)?;
    let (client_pem, client_key_pem) = leaf(&san, "zhuanfa-node", &ca_cert, &ca_key)?;

    fs::write(&paths.ca, ca_cert.pem())?;
    fs::write(&paths.server, server_pem)?;
    fs::write(&paths.server_key, server_key_pem)?;
    fs::write(&paths.client, client_pem)?;
    fs::write(&paths.client_key, client_key_pem)?;
    Ok(paths)
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
