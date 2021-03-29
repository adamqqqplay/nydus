// Copyright 2020 Ant Group. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

//! Trace image building procedure

use std::any::Any;
use std::cmp::{Eq, PartialEq};
use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::sync::{atomic::AtomicU64, Arc, Mutex, RwLock};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::{error::Error, value::Value};

impl Display for TraceClass {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            TraceClass::Timing => write!(f, "consumed_time"),
            TraceClass::Event => write!(f, "registered_events"),
        }
    }
}

macro_rules! enum_str {
    ($m:meta
    pub enum $name:ident {
        $($variant:ident = $val:expr),*,
    }) => {
        #[$m]
        pub enum $name {
            $($variant = $val),*
        }

        impl $name {
            fn name(&self) -> String {
                match self {
                    $($name::$variant => format!("{}", $name::$variant)),*
                }
            }
        }
    };
}

enum_str! {
derive(Hash, Eq, PartialEq)
pub enum TraceClass {
    Timing = 1,
    Event = 2,
}
}
pub enum TraceError {
    Serde(Error),
}

type Result<T> = std::result::Result<T, TraceError>;

/// Used to measure time consuming and gather all tracing points when building image.
#[derive(Serialize, Default)]
pub struct TimingTracerClass {
    // Generally speaking, we won't have many timing tracers act from multiple points.
    // So `Mutex` should fill our requirements.
    #[serde(flatten)]
    records: Mutex<HashMap<String, f32>>,
}

pub trait TracerClass: Send + Sync + 'static {
    fn release(&self) -> Result<Value>;
    fn as_any(&self) -> &dyn Any;
}

impl TracerClass for TimingTracerClass {
    fn release(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(TraceError::Serde)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn trace_timing<F: FnOnce() -> T, T>(point: &str, tracer: &TimingTracerClass, f: F) -> T {
    let begin = SystemTime::now();
    let r = f();
    let elapsed = SystemTime::now().duration_since(begin).unwrap();

    // Not expect poisoned lock.
    tracer
        .records
        .lock()
        .unwrap()
        .insert(point.to_string(), elapsed.as_secs_f32());

    r
}

/// The root tracer manages all kinds of tracers registered to it.
/// The statistics/events/records can be printed out or persisted from the root
/// tracer. When building procedure is finished, root tracer can dump all tracing
/// points to specified output file.
pub struct BuildRootTracer {
    tracers: RwLock<HashMap<TraceClass, Arc<dyn TracerClass>>>,
}

impl BuildRootTracer {
    pub fn register(&self, class: TraceClass, tracer: Arc<dyn TracerClass>) {
        let mut guard = self.tracers.write().unwrap();
        guard.insert(class, tracer);
    }

    pub fn tracer(&self, class: TraceClass) -> Arc<dyn TracerClass> {
        let g = self.tracers.read().unwrap();
        // Safe to unwrap because tracers should always be enabled
        (&g).get(&class).unwrap().clone()
    }

    pub fn dump_summary_map(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let mut map = serde_json::Map::new();
        for c in self.tracers.write().unwrap().iter() {
            map.insert(c.0.name(), c.1.release()?);
        }
        Ok(map)
    }
}

#[derive(Serialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum TraceEvent {
    Counter(AtomicU64),
    Fixed(u64),
    Desc(String),
}

#[derive(Serialize, Default)]
pub struct EventTracerClass {
    #[serde(flatten)]
    pub events: RwLock<HashMap<String, TraceEvent>>,
}

impl TracerClass for EventTracerClass {
    fn release(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(TraceError::Serde)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

lazy_static! {
    pub static ref BUILDING_RECORDER: BuildRootTracer = BuildRootTracer {
        tracers: RwLock::new(HashMap::default())
    };
}

macro_rules! root_tracer {
    () => {
        &crate::trace::BUILDING_RECORDER as &$crate::trace::BuildRootTracer
    };
}

#[macro_export]
macro_rules! timing_tracer {
    () => {
        root_tracer!()
            .tracer($crate::trace::TraceClass::Timing)
            .as_any()
            .downcast_ref::<$crate::trace::TimingTracerClass>()
            .unwrap()
    };
    ($f:block, $key:expr) => {
        $crate::trace::trace_timing($key, timing_tracer!(), || $f)
    };
    ($f:block, $key:expr, $t:ty) => {
        $crate::trace::trace_timing::<_, $t>($key, timing_tracer!(), || $f)
    };
}

#[macro_export]
macro_rules! register_tracer {
    ($class:expr, $r:ty) => {
        root_tracer!().register($class, Arc::new(<$r>::default()));
    };
}

#[macro_export]
macro_rules! event_tracer {
    () => {
        root_tracer!()
            .tracer($crate::trace::TraceClass::Event)
            .as_any()
            .downcast_ref::<$crate::trace::EventTracerClass>()
            .unwrap()
    };
    ($event:expr, $desc:expr) => {
        event_tracer!().events.write().unwrap().insert(
            $event.to_string(),
            $crate::trace::TraceEvent::Fixed($desc as u64),
        )
    };
    ($event:expr, +$value:expr) => {
        use std::sync::atomic::{AtomicU64, Ordering};
        let mut new: bool = true;
        {
            if let Some($crate::trace::TraceEvent::Counter(ref e)) =
                event_tracer!().events.read().unwrap().get($event)
            {
                e.fetch_add($value as u64, Ordering::Relaxed);
                new = false;
            }
        }

        if new {
            // Double check to close the race that another thread has already inserted.
            if let Ok(ref mut guard) = event_tracer!().events.write() {
                if let Some($crate::trace::TraceEvent::Counter(ref e)) = guard.get($event) {
                    e.fetch_add($value as u64, Ordering::Relaxed);
                } else {
                    guard.insert(
                        $event.to_string(),
                        $crate::trace::TraceEvent::Counter(AtomicU64::new(0)),
                    );
                }
            }
        }
    };
    ($event:expr, $format:expr, $value:expr) => {
        if let Ok(ref mut guard) = event_tracer!().events.write() {
            guard.insert(
                $event.to_string(),
                $crate::trace::TraceEvent::Desc(format!($format, $value)),
            );
        }
    };
}
