// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! A `ureq` DNS resolver that never calls glibc's `getaddrinfo`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::Deserialize;
use ureq::Error;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

/// Cloudflare's `DoH` server, addressed by IP so this bootstrap request never itself
/// needs a name lookup.
const DOH_SERVER: &str = "1.1.1.1";

#[derive(Debug, Default)]
pub(crate) struct DohResolver;

impl Resolver for DohResolver {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let scheme = uri.scheme().ok_or_else(|| Error::BadUri(uri.to_string()))?;
        let authority = uri
            .authority()
            .ok_or_else(|| Error::BadUri(uri.to_string()))?;
        let port = authority.port_u16().or(match scheme.as_str() {
            "https" => Some(443),
            "http" => Some(80),
            _ => None,
        });
        let port = port.ok_or_else(|| Error::BadUri(uri.to_string()))?;
        let host = authority.host();

        let mut result = self.empty();

        if let Ok(ip) = host.parse::<IpAddr>() {
            result.push(SocketAddr::new(ip, port));
            return Ok(result);
        }

        for ip in doh_lookup(host)? {
            result.push(SocketAddr::new(IpAddr::V4(ip), port));
        }
        if result.is_empty() {
            return Err(Error::HostNotFound);
        }
        Ok(result)
    }
}

#[derive(Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer", default)]
    answer: Vec<DohAnswer>,
}

#[derive(Deserialize)]
struct DohAnswer {
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

fn doh_lookup(host: &str) -> Result<Vec<Ipv4Addr>, Error> {
    let url = format!("https://{DOH_SERVER}/dns-query?name={host}&type=A");
    let body = ureq::get(&url)
        .header("accept", "application/dns-json")
        .call()?
        .body_mut()
        .read_to_string()?;
    let response: DohResponse =
        serde_json::from_str(&body).map_err(|e| Error::Other(Box::new(e)))?;
    Ok(response
        .answer
        .into_iter()
        .filter(|a| a.kind == 1)
        .filter_map(|a| a.data.parse().ok())
        .collect())
}
