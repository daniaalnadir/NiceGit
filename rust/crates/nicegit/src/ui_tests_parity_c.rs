//! Interface tests for the remaining details of Mac app features, built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;
