//! Python plugin.
//!
//! Embeds a Python interpreter in the GUI application, providing:
//! - `python` command (alias `/`) for inline Python execution
//! - `.py` file handler for `run script.py`
//! - `sys._patinae_backend` for embedded `patinae` package access

mod atom_ops;
mod backend;
mod commands;
mod engine;
mod handler;
mod highlight;
mod panel;
mod runtime;
mod shared;
mod worker;

use patinae_plugin::patinae_plugin;

patinae_plugin! {
    name: "python",
    description: "Embedded Python interpreter for scripting and automation",
    commands: [],
    background_registration: true,
    register: |reg| {
        runtime::register(reg);
    },
}
