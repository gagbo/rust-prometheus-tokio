// Copyright 2023 Gerry Agbobada. Licensed under Apache-2.0.

//! Monitor a Tokio Runtime, collecting metrics for [Prometheus](prometheus).
//!
//! This module needs the `tokio_unstable` RUSTFLAG flag to work properly.

use prometheus::{
    core::Desc,
    core::{Collector, Opts},
    proto, CounterVec, Gauge, IntCounter, IntCounterVec, IntGauge, IntGaugeVec,
};
use tokio::runtime::{Handle, RuntimeMetrics};

const ALL_METRICS: [(&str, &str); 15] = [
    WORKERS_COUNT,
    PARK_COUNT,
    NOOP_COUNT,
    STEAL_COUNT,
    STEAL_OPERATIONS,
    REMOTE_SCHEDULE_COUNT,
    LOCAL_SCHEDULE_COUNT,
    OVERFLOW_COUNT,
    POLLS_COUNT,
    BUSY_DURATION_SECONDS,
    INJECTION_QUEUE_DEPTH_GAUGE,
    LOCAL_QUEUE_DEPTH_GAUGE,
    BUDGET_FORCED_YIELD_COUNT,
    IO_DRIVER_READY_COUNT,
    MEAN_POLLS_PER_PARK,
];

const METRICS_NUMBER: usize = ALL_METRICS.len();

const WORKER_LABEL: &str = "worker";

const WORKERS_COUNT: (&str, &str) = (
    "rust_tokio_workers_count",
    r#"The number of worker threads used by the runtime.
This metric is static for a runtime.
This metric is always equal to tokio::runtime::RuntimeMetrics::num_workers. When using the current_thread runtime, the return value is always 1."#,
);
const PARK_COUNT: (&str, &str) = (
    "rust_tokio_parks",
    r#"The number of times worker threads parked.
The worker park count increases by one each time the worker parks the thread waiting for new inbound events to process. This usually means the worker has processed all pending work and is currently idle."#,
);
const NOOP_COUNT: (&str, &str) = (
    "rust_tokio_noops",
    r#"The number of times worker threads unparked but performed no work before parking again.
The worker no-op count increases by one each time the worker unparks the thread but finds no new work and goes back to sleep. This indicates a false-positive wake up."#,
);
const STEAL_COUNT: (&str, &str) = (
    "rust_tokio_steals",
    r#"The number of tasks worker threads stole from another worker thread.
The worker steal count increases by the amount of stolen tasks each time the worker has processed its scheduled queue and successfully steals more pending tasks from another worker.
This metric only applies to the multi-threaded runtime and will always return 0 when using the current thread runtime."#,
);
const STEAL_OPERATIONS: (&str, &str) = (
    "rust_tokio_steal_operations",
    r#"The number of times worker threads stole tasks from another worker thread.
The worker steal operations increases by one each time the worker has processed its scheduled queue and successfully steals more pending tasks from another worker.
This metric only applies to the multi-threaded runtime and will always return 0 when using the current thread runtime."#,
);
const REMOTE_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_remote_schedules",
    r#"The number of tasks scheduled from outside of the runtime.
The remote schedule count increases by one each time a task is woken from outside of the runtime. This usually means that a task is spawned or notified from a non-runtime thread and must be queued using the Runtime’s injection queue, which tends to be slower."#,
);
const LOCAL_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_local_schedules",
    r#"The number of tasks scheduled from worker threads.
The local schedule count increases by one each time a task is woken from inside of the runtime. This usually means that a task is spawned or notified from within a runtime thread and will be queued on the worker-local queue."#,
);
const OVERFLOW_COUNT: (&str, &str) = (
    "rust_tokio_overflows",
    r#"The number of times worker threads saturated their local queues.
The worker steal count increases by one each time the worker attempts to schedule a task locally, but its local queue is full. When this happens, half of the local queue is moved to the injection queue.
This metric only applies to the multi-threaded scheduler."#,
);
const POLLS_COUNT: (&str, &str) = (
    "rust_tokio_polls",
    r#"The number of tasks that have been polled across all worker threads.
The worker poll count increases by one each time a worker polls a scheduled task."#,
);
const BUSY_DURATION_SECONDS: (&str, &str) = (
    "rust_tokio_busy_duration_seconds",
    r#"The amount of time worker threads were busy.
The worker busy duration increases whenever the worker is spending time processing work. Using this value can indicate the  load of workers."#,
);
const INJECTION_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_injection_queue_depth_gauge",
    r#"The number of tasks currently scheduled in the runtime’s injection queue.
Tasks that are spawned or notified from a non-runtime thread are scheduled using the runtime’s injection queue. This metric returns the current number of tasks pending in the injection queue. As such, the returned value may increase or decrease as new tasks are scheduled and processed."#,
);
const LOCAL_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_local_queue_depth_gauge",
    r#"The number of tasks currently scheduled in workers’ local queues.
Tasks that are spawned or notified from within a runtime thread are scheduled using that worker’s local queue. This metric returns the current number of tasks pending in all workers’ local queues. As such, the returned value may increase or decrease as new tasks are scheduled and processed."#,
);
const BUDGET_FORCED_YIELD_COUNT: (&str, &str) = (
    "rust_tokio_budget_forced_yield_count",
    r#"The number of times that tasks have been forced to yield back to the scheduler after exhausting their task budgets.
This count starts at zero when the runtime is created and increases by one each time a task yields due to exhausting its budget.
The counter is monotonically increasing. It is never decremented or reset to zero."#,
);
const IO_DRIVER_READY_COUNT: (&str, &str) = (
    "rust_tokio_io_driver_ready_count",
    r#"The number of ready events processed by the runtime’s I/O driver.
The metric needs the 'net' feature from tokio to be reported."#,
);
const MEAN_POLLS_PER_PARK: (&str, &str) = (
    "rust_tokio_mean_polls_per_park",
    r#"Mean number of tasks that have been polled per useful worker park event.

A useful park event is a park event that was not a noop for the worker. If there are no useful park events, then the metric returns NaN."#,
);

/// A collector which exports the current state of tokio runtime metrics.
#[derive(Debug)]
pub struct TokioCollector {
    metrics: RuntimeMetrics,
    descs: Vec<Desc>,
    workers: IntGauge,
    parks: IntCounterVec,
    noops: IntCounterVec,
    stolen_tasks: IntCounterVec,
    steals: IntCounterVec,
    remote_schedules: IntCounter,
    local_schedules: IntCounterVec,
    overflows: IntCounterVec,
    polls: IntCounterVec,
    busy_duration: CounterVec,
    injection_queue_depth: IntGauge,
    local_queue_depth: IntGaugeVec,
    budget_forced_yield_tasks: IntCounter,
    #[cfg_attr(not(feature = "io_driver"), allow(dead_code))]
    io_driver_ready_events: IntCounter,
    mean_polls_per_park: Gauge,
}

impl TokioCollector {
    /// Create a new collector from the given runtime handle.
    pub fn new<S: Into<String>>(handle: &Handle, namespace: S) -> Self {
        let namespace = namespace.into();
        let mut descs = Vec::new();

        let workers = IntGauge::with_opts(
            Opts::new(WORKERS_COUNT.0, WORKERS_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(workers.desc().into_iter().cloned());

        let parks = IntCounterVec::new(
            Opts::new(PARK_COUNT.0, PARK_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(parks.desc().into_iter().cloned());

        let noops = IntCounterVec::new(
            Opts::new(NOOP_COUNT.0, NOOP_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(noops.desc().into_iter().cloned());

        let stolen_tasks = IntCounterVec::new(
            Opts::new(STEAL_COUNT.0, STEAL_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(stolen_tasks.desc().into_iter().cloned());

        let steals = IntCounterVec::new(
            Opts::new(STEAL_OPERATIONS.0, STEAL_OPERATIONS.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(steals.desc().into_iter().cloned());

        let remote_schedules = IntCounter::with_opts(
            Opts::new(REMOTE_SCHEDULE_COUNT.0, REMOTE_SCHEDULE_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(remote_schedules.desc().into_iter().cloned());

        let local_schedules = IntCounterVec::new(
            Opts::new(LOCAL_SCHEDULE_COUNT.0, LOCAL_SCHEDULE_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(local_schedules.desc().into_iter().cloned());

        let overflows = IntCounterVec::new(
            Opts::new(OVERFLOW_COUNT.0, OVERFLOW_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(overflows.desc().into_iter().cloned());

        let polls = IntCounterVec::new(
            Opts::new(POLLS_COUNT.0, POLLS_COUNT.1).namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(polls.desc().into_iter().cloned());

        let busy_duration = CounterVec::new(
            Opts::new(BUSY_DURATION_SECONDS.0, BUSY_DURATION_SECONDS.1)
                .namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(busy_duration.desc().into_iter().cloned());

        let injection_queue_depth = IntGauge::with_opts(
            Opts::new(INJECTION_QUEUE_DEPTH_GAUGE.0, INJECTION_QUEUE_DEPTH_GAUGE.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(injection_queue_depth.desc().into_iter().cloned());

        let local_queue_depth = IntGaugeVec::new(
            Opts::new(LOCAL_QUEUE_DEPTH_GAUGE.0, LOCAL_QUEUE_DEPTH_GAUGE.1)
                .namespace(namespace.clone()),
            &[WORKER_LABEL],
        )
        .unwrap();
        descs.extend(local_queue_depth.desc().into_iter().cloned());

        let budget_forced_yield_tasks = IntCounter::with_opts(
            Opts::new(BUDGET_FORCED_YIELD_COUNT.0, BUDGET_FORCED_YIELD_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(budget_forced_yield_tasks.desc().into_iter().cloned());

        let io_driver_ready_events = IntCounter::with_opts(
            Opts::new(IO_DRIVER_READY_COUNT.0, IO_DRIVER_READY_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(io_driver_ready_events.desc().into_iter().cloned());

        let mean_polls_per_park = Gauge::with_opts(
            Opts::new(MEAN_POLLS_PER_PARK.0, MEAN_POLLS_PER_PARK.1).namespace(namespace),
        )
        .unwrap();
        descs.extend(mean_polls_per_park.desc().into_iter().cloned());

        let metrics = handle.metrics();

        Self {
            metrics,
            descs,
            workers,
            remote_schedules,
            injection_queue_depth,
            budget_forced_yield_tasks,
            io_driver_ready_events,
            mean_polls_per_park,
            parks,
            noops,
            stolen_tasks,
            steals,
            local_schedules,
            overflows,
            polls,
            busy_duration,
            local_queue_depth,
        }
    }

    pub fn for_self() -> Self {
        let handle = tokio::runtime::Handle::current();
        Self::new(&handle, "")
    }
}

impl Collector for TokioCollector {
    fn desc(&self) -> Vec<&Desc> {
        self.descs.iter().collect()
    }

    fn collect(&self) -> Vec<proto::MetricFamily> {
        tracing::trace!("Start collecting Tokio metrics");
        // collect MetricFamilys.
        let mut mfs = Vec::with_capacity(METRICS_NUMBER);

        tracing::trace!("Collect numbers");
        let count = self.metrics.num_workers();
        self.workers.set(count as i64);
        mfs.extend(self.workers.collect());

        let mut old_polls_total: u64 = 0;
        let mut new_polls_total: u64 = 0;
        let mut old_parks_total: u64 = 0;
        let mut new_parks_total: u64 = 0;
        let mut old_noops_total: u64 = 0;
        let mut new_noops_total: u64 = 0;
        for worker in 0..count {
            let worker_label = worker.to_string();
            macro_rules! per_worker_counter {
                ( $field:ident, $rt_metric:ident, "int" ) => {{
                    let past = self.$field.with_label_values(&[&worker_label]).get() as u64;
                    let new = self.metrics.$rt_metric(worker) as u64;
                    debug_assert!(new >= past, "new: {new} >= past: {past}");
                    self.$field
                        .with_label_values(&[&worker_label])
                        .inc_by(new.saturating_sub(past));
                }};
                ( $field:ident, $rt_metric:ident, "duration" ) => {{
                    let past = self.$field.with_label_values(&[&worker_label]).get();
                    let new = self.metrics.$rt_metric(worker).as_secs_f64();
                    debug_assert!(new >= past, "new: {new} >= past: {past}");
                    self.$field
                        .with_label_values(&[&worker_label])
                        .inc_by(new - past);
                }};
            }

            macro_rules! per_worker_gauge {
                ( $field:ident, $rt_metric:ident, "int" ) => {{
                    let new = self.metrics.$rt_metric(worker) as i64;
                    self.$field.with_label_values(&[&worker_label]).set(new);
                }};
            }

            old_polls_total += self
                .polls
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();
            old_parks_total += self
                .parks
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();
            old_noops_total += self
                .noops
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();

            per_worker_counter!(parks, worker_park_count, "int");
            per_worker_counter!(noops, worker_noop_count, "int");
            per_worker_counter!(stolen_tasks, worker_steal_count, "int");
            per_worker_counter!(steals, worker_steal_operations, "int");
            per_worker_counter!(local_schedules, worker_local_schedule_count, "int");
            per_worker_counter!(overflows, worker_overflow_count, "int");
            per_worker_counter!(polls, worker_poll_count, "int");
            per_worker_gauge!(local_queue_depth, worker_local_queue_depth, "int");
            per_worker_counter!(busy_duration, worker_total_busy_duration, "duration");

            new_polls_total += self
                .polls
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();
            new_parks_total += self
                .parks
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();
            new_noops_total += self
                .noops
                .get_metric_with_label_values(&[&worker_label])
                .unwrap()
                .get();
        }

        mfs.extend(self.parks.collect());
        mfs.extend(self.noops.collect());
        mfs.extend(self.stolen_tasks.collect());
        mfs.extend(self.steals.collect());
        mfs.extend(self.local_schedules.collect());
        mfs.extend(self.overflows.collect());
        mfs.extend(self.polls.collect());
        mfs.extend(self.local_queue_depth.collect());
        mfs.extend(self.busy_duration.collect());

        {
            let past = self.remote_schedules.get();
            let new = self.metrics.remote_schedule_count();
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.remote_schedules.inc_by(new.saturating_sub(past));
            mfs.extend(self.remote_schedules.collect());
        }

        {
            let past = self.budget_forced_yield_tasks.get();
            let new = self.metrics.budget_forced_yield_count();
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.budget_forced_yield_tasks
                .inc_by(new.saturating_sub(past));
            mfs.extend(self.budget_forced_yield_tasks.collect());
        }

        {
            let new = self.metrics.injection_queue_depth() as i64;
            self.injection_queue_depth.set(new);
            mfs.extend(self.injection_queue_depth.collect())
        }

        #[cfg(feature = "io_driver")]
        {
            let past = self.io_driver_ready_events.get();
            let new = self.metrics.io_driver_ready_count();
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.io_driver_ready_events.inc_by(new.saturating_sub(past));
            mfs.extend(self.io_driver_ready_events.collect());
        }
        #[cfg(not(feature = "io_driver"))]
        {}

        {
            let delta_parks = new_parks_total.saturating_sub(old_parks_total);
            let delta_noops = new_noops_total.saturating_sub(old_noops_total);
            let delta_polls = new_polls_total.saturating_sub(old_polls_total);
            debug_assert!(delta_parks >= delta_noops, "There are at least as many parks as noops (Δparks = {delta_parks}, Δnoops = {delta_noops})");
            let useful_parks = delta_parks.saturating_sub(delta_noops);
            if useful_parks == 0 {
                self.mean_polls_per_park.set(f64::NAN);
            } else {
                self.mean_polls_per_park
                    .set(delta_polls as f64 / useful_parks as f64);
            }
            mfs.extend(self.mean_polls_per_park.collect());
        }

        tracing::trace!("Done Collecting numbers");

        mfs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::{core::Collector, Registry};
    use tokio::runtime;

    #[test]
    fn test_process_collector() {
        let rt = runtime::Builder::new_current_thread().build().unwrap();
        let tc = TokioCollector::new(rt.handle(), "");
        {
            let descs = tc.desc();
            assert_eq!(descs.len(), METRICS_NUMBER);

            let mfs = tc.collect();
            // Without the io_driver feature, the io_driver_ready_count related
            // metric is not reported
            #[cfg(not(feature = "io_driver"))]
            assert_eq!(mfs.len(), METRICS_NUMBER - 1);
            #[cfg(feature = "io_driver")]
            assert_eq!(mfs.len(), METRICS_NUMBER);
        }

        let r = Registry::new();
        let res = r.register(Box::new(tc));
        assert!(res.is_ok());
    }
}
