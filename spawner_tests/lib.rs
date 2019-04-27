extern crate cfg_if;

use cfg_if::cfg_if;

#[cfg(test)]
mod common;

cfg_if! {
    if #[cfg(test)] {
        extern crate rand;
        extern crate spawner;
        extern crate spawner_driver;

        mod term_reason;
        mod redirect;
        mod env;
        mod protocol;
        mod other;
        mod error;
        mod resource_usage;
    }
}
