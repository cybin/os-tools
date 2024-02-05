// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Operations that happen post-blit (primarily, triggers within container)
//! Note that we support transaction scope and system scope triggers, invoked
//! before `/usr` is activated and after, respectively.
//!
//! Note that currently we only load from `/usr/share/moss/triggers/{tx,sys.d}/*.yaml`
//! and do not yet support local triggers
use std::process;

use container::Container;
use serde::Deserialize;
use thiserror::Error;
use triggers::{
    format::{Handler, Trigger},
    TriggerCommand,
};

use crate::Installation;

use super::{create_root_links, PendingFile};

/// Transaction triggers
/// These are loaded from `/usr/share/moss/triggers/tx.d/*.yaml`
#[derive(Deserialize, Debug)]
struct TransactionTrigger(Trigger);

impl config::Config for TransactionTrigger {
    fn domain() -> String {
        "tx".into()
    }
}

/// Handle all postblit tasks
pub async fn postblit(
    fstree: vfs::tree::Tree<PendingFile>,
    install: &Installation,
) -> Result<(), Error> {
    create_root_links(&install.isolation_dir()).await?;

    // Load all pre.d triggers
    let datadir = install
        .staging_path("usr")
        .join("share")
        .join("moss")
        .join("triggers");
    let triggers = config::Manager::custom(datadir)
        .load::<TransactionTrigger>()
        .await;

    // Push all transaction paths into the postblit trigger collection
    let mut manager = triggers::Collection::new(triggers.iter().map(|t| &t.0))?;
    manager.process_paths(fstree.iter().map(|m| m.to_string()));
    let computed_triggers = manager.bake()?;

    // Execute in dependency order
    for trigger in computed_triggers.iter() {
        execute_trigger(install, trigger)?;
    }

    Ok(())
}

/// Entry point for the execution of a trigger
///
/// We expect each trigger to run in a virtual filesystem that uses the current
/// `staging_dir` for the `/usr` tree, and execution is performed within a clone-based
/// container.
fn execute_trigger(install: &Installation, trigger: &TriggerCommand) -> Result<(), Error> {
    let isolation = Container::new(install.isolation_dir())
        .networking(false)
        .override_accounts(false)
        .bind_ro(install.root.join("etc"), "/etc")
        .bind_rw(install.staging_path("usr"), "/usr")
        .work_dir("/");

    Ok(isolation.run(|| execute_trigger_internal(trigger))?)
}

/// Internal executor for triggers.
fn execute_trigger_internal(trigger: &TriggerCommand) -> Result<(), Error> {
    eprintln!("Trigger: {:?}", trigger);
    match &trigger.handler {
        Handler::Run { run, args } => {
            let cmd = process::Command::new(run)
                .args(args)
                .current_dir("/")
                .output()?;

            if let Some(code) = cmd.status.code() {
                if code != 0 {
                    eprintln!("Trigger exited with non-zero status code: {run} {args:?}");
                    eprintln!("   Stodut: {}", String::from_utf8(cmd.stdout).unwrap());
                    eprintln!("   Stderr: {}", String::from_utf8(cmd.stderr).unwrap());
                }
            } else {
                eprintln!("Failed to execute trigger: {run} {args:?}");
            }
        }
        Handler::Delete { delete } => todo!(),
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("container")]
    Container(#[from] container::Error),

    #[error("triggers")]
    Triggers(#[from] triggers::Error),

    #[error("io")]
    IO(#[from] std::io::Error),
}
