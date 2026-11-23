#![forbid(unsafe_code)]
//! Physics-inspired digital twin for granite impact-modal NDT.
//!
//! The twin synthesises a laser-vibrometer signal from a simply-supported
//! rectangular plate model, perturbs it with phenomenological defects, passes
//! it through a sensor/ADC model, packetises it with the real wire protocol
//! and finally feeds it through the production stream assembler, analysis
//! pipeline and classifier.  It is therefore a closed-loop test bench for the
//! entire system before the physical instrument exists.
//!
//! ```no_run
//! let run = granite_sim::run("healthy").unwrap();
//! println!("{} -> {}", run.scenario.title, run.assessment.condition);
//! ```

pub mod defect;
pub mod lab;
pub mod material;
pub mod network;
pub mod plate;
pub mod scenario;
pub mod signal;
pub mod suite;
pub mod visual;

pub use defect::{Defect, DefectKind};
pub use lab::{render_lab_html, scene_from_run, write_lab, LabScene};
pub use material::{Material, CONCRETE, GRANITE, MARBLE};
pub use network::{Datagram, NetworkConfig, ReceiveResult};
pub use plate::{PlateGeometry, PlateMode, PlateModel, Point2};
pub use scenario::{catalog, find, healthy_reference, run, run_scenario, Scenario, ScenarioRun};
pub use signal::{dominant_modes, modal_response, sensor_chain, synthesize, SensorConfig, Tap};
pub use suite::{run_suite, write_bundle, ReportBundle, SuiteOptions, SuiteResult};
pub use visual::{render_scope_html, render_showcase_html, write_scope, write_showcase};
