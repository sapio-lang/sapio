// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde_json::Value;
use tokio::{
    select,
    sync::{
        broadcast,
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
};

use crate::contracts::Request;

use super::{CommandReturn, RequestError, Response};

pub struct Server {
    chan: UnboundedReceiver<(Request, oneshot::Sender<Response>)>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
}

impl Server {
    pub fn new() -> (
        Self,
        UnboundedSender<(Request, oneshot::Sender<Response>)>,
        broadcast::Sender<()>,
    ) {
        let (a, b) = unbounded_channel();
        let (c, d) = broadcast::channel(1);
        let s = Server {
            chan: b,
            shutdown: d,
        };
        (s, a, c)
    }
    pub fn run(mut self) {
        tokio::spawn(async move {
            let mut shutdown = self.shutdown;
            'terminate: loop {
                select! {
                    _ = shutdown.recv() => {
                        break 'terminate;
                    }
                    Some((req, resp)) = self.chan.recv() => {
                        resp.send(req.handle().await);
                    }
                }
            }
        });
    }
}
