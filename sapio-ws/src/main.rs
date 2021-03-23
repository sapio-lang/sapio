// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[deny(missing_docs)]
use actix::{Actor, StreamHandler};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;

use sapio_contrib::contracts;
use sapio_front::session;

/// Define HTTP actor
struct MyWs {
    sesh: session::Session,
}

impl Actor for MyWs {
    type Context = ws::WebsocketContext<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        let r = self.sesh.open();
        ctx.text(r);
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWs {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        let bm = &msg;
        let m = match bm {
            Ok(ws::Message::Text(text)) => Ok(session::Msg::Text(&text)),
            Ok(ws::Message::Binary(bin)) => Ok(session::Msg::Bytes(&bin)),
            _ => Err(()),
        };
        if let Ok(m) = m {
            if let Ok(Some(Ok(s))) = self
                .sesh
                .handle(m)
                .map(|v| v.map(|v2| serde_json::to_string(&v2)))
            {
                ctx.text(s);
                return;
            }
        }

        ctx.close(None);
    }
}

async fn index(
    m: &'static session::Menu,
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    let resp = ws::start(
        MyWs {
            sesh: session::Session::new(m, bitcoin::Network::Regtest),
        },
        &req,
        stream,
    );
    println!("{:?}", resp);
    resp
}

lazy_static::lazy_static! {
    static ref MENU : session::Menu = {
        let mut m = session::MenuBuilder::new();
        m.register_as::<contracts::treepay::TreePay>("TreePay".to_string().into());
        m.register_as::<contracts::hodl_chicken::HodlChickenInner>("HodlChicken".to_string().into());
        // Readme Contracts
        m.register_as::<contracts::readme_contracts::PayToPublicKey>("P2PK".to_string().into());
        m.register_as::<contracts::readme_contracts::BasicEscrow>("BasicEscrow".to_string().into());
        m.register_as::<contracts::readme_contracts::BasicEscrow2>("BasicEscrow2".to_string().into());
        m.register_as::<contracts::readme_contracts::TrustlessEscrow>("TrustlessEscrow".to_string().into());

        m.register_as_from::<contracts::vault::VaultAddress, contracts::vault::Vault, _>("Vault->Address".to_string().into());
        m.register_as_from::<contracts::vault::VaultTree, contracts::vault::Vault, _>("Vault->TreePay".to_string().into());

        m.register_as::<contracts::federated_sidechain::PegIn>("FederatedPegIn".to_string().into());

        m.into()
    };
}
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().route("/", web::get().to(|r, s| index(&MENU, r, s))))
        .bind("127.0.0.1:8888")?
        .run()
        .await
}
