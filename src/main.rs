// Copyright 2024 Cloudflare, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::{atomic::AtomicUsize, Arc};

use clap::Parser;
use pingora::{
    connectors::{http::Connector, ConnectorOptions},
    prelude::*,
    protocols::http::client::HttpSession,
};

#[derive(Parser)]
struct Opts {
    addr: String,
    #[clap(short = 'c', long, default_value = "500")]
    num_connection: usize,
    #[clap(short = 'n', long, default_value = "1000000")]
    num_works: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let opts = Opts::parse();

    let counter = Arc::new(AtomicUsize::new(0));
    let connector_options = ConnectorOptions::new(opts.num_connection);
    let connector = Arc::new(Connector::new(Some(connector_options)));

    let now = std::time::Instant::now();
    let futures: Vec<_> = (0..opts.num_connection)
        .map(|_| {
            let counter = counter.clone();
            let connector = connector.clone();
            let peer_addr = opts.addr.clone();
            tokio::spawn(async move {
                // create the HTTP session
                let mut peer = HttpPeer::new(peer_addr, false, "localhost".into());
                peer.options.set_http_version(1, 1);
                // dbg!(peer.reuse_hash());

                // perform a GET request
                while counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < opts.num_works {
                    let (mut http, _reused) = connector.get_http_session(&peer).await?;
                    let mut new_request = RequestHeader::build("GET", b"/", None)?;
                    new_request.append_header("Connection", "keep-alive")?;

                    http.write_request_header(Box::new(new_request)).await?;

                    // Servers usually don't respond until the full request body is read.
                    http.finish_request_body().await?;
                    http.read_response_header().await?;
                    while let Some(_chunk) = http.read_response_body().await? {}

                    match &mut http {
                        HttpSession::H1(h1) => {
                            h1.respect_keepalive();
                        }
                        HttpSession::H2(_h2) => {}
                    }
                    connector
                        .release_http_session(http, &peer, Some(std::time::Duration::from_secs(5)))
                        .await;
                }

                Ok::<(), pingora::BError>(())
            })
        })
        .collect();

    for f in futures {
        let _ = f.await;
    }
    let elapsed = now.elapsed();

    println!(
        "Finished {} requests in {:?}, {:.2} req/s",
        opts.num_works,
        elapsed,
        opts.num_works as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
