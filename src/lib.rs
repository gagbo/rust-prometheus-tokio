// Copyright 2023 Gerry Agbobada. Licensed under Apache-2.0.

//! Monitor a Tokio Runtime, collecting metrics for [Prometheus](prometheus).
//!
//! This module needs the `tokio_unstable` RUSTFLAG flag to work properly.

use const_format::formatcp;
use prometheus::{
    core::Desc,
    core::{Collector, Opts},
    proto, Counter, Gauge, IntCounter, IntGauge,
};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio_metrics::{RuntimeIntervals, RuntimeMonitor};

const ALL_METRICS: [(&str, &str); 34] = [
    WORKERS_COUNT,
    TOTAL_PARK_COUNT,
    MIN_PARK_COUNT,
    MAX_PARK_COUNT,
    TOTAL_NOOP_COUNT,
    MAX_NOOP_COUNT,
    MIN_NOOP_COUNT,
    TOTAL_STEAL_COUNT,
    MAX_STEAL_COUNT,
    MIN_STEAL_COUNT,
    TOTAL_STEAL_OPERATIONS,
    MAX_STEAL_OPERATIONS,
    MIN_STEAL_OPERATIONS,
    REMOTE_SCHEDULE_COUNT,
    TOTAL_LOCAL_SCHEDULE_COUNT,
    MAX_LOCAL_SCHEDULE_COUNT,
    MIN_LOCAL_SCHEDULE_COUNT,
    TOTAL_OVERFLOW_COUNT,
    MAX_OVERFLOW_COUNT,
    MIN_OVERFLOW_COUNT,
    TOTAL_POLLS_COUNT,
    MAX_POLLS_COUNT,
    MIN_POLLS_COUNT,
    TOTAL_BUSY_DURATION_SECONDS,
    MAX_BUSY_DURATION_SECONDS,
    MIN_BUSY_DURATION_SECONDS,
    INJECTION_QUEUE_DEPTH_GAUGE,
    TOTAL_LOCAL_QUEUE_DEPTH_GAUGE,
    MAX_LOCAL_QUEUE_DEPTH_GAUGE,
    MIN_LOCAL_QUEUE_DEPTH_GAUGE,
    BUDGET_FORCED_YIELD_COUNT,
    IO_DRIVER_READY_COUNT,
    MEAN_POLLS_PER_PARK,
    BUSY_RATIO,
];

const METRICS_NUMBER: usize = ALL_METRICS.len();

// TODO: Review the naming conventions for all the metrics.
//   "total", "min", and "max" must almost always be in suffix positions
const WORKERS_COUNT: (&str, &str) = (
    "rust_tokio_workers_count",
    r#"The number of worker threads used by the runtime.
This metric is static for a runtime.
This metric is always equal to tokio::runtime::RuntimeMetrics::num_workers. When using the current_thread runtime, the return value is always 1."#,
);
const TOTAL_PARK_COUNT: (&str, &str) = (
    "rust_tokio_parks_total",
    r#"The number of times worker threads parked.
The worker park count increases by one each time the worker parks the thread waiting for new inbound events to process. This usually means the worker has processed all pending work and is currently idle."#,
);
const MIN_PARK_COUNT: (&str, &str) = (
    "rust_tokio_parks_min",
    r#"The minimum number of times any worker thread parked."#,
);
const MAX_PARK_COUNT: (&str, &str) = (
    "rust_tokio_parks_max",
    r#"The maximum number of times any worker thread parked."#,
);
const TOTAL_NOOP_COUNT: (&str, &str) = (
    "rust_tokio_noops_total",
    r#"The number of times worker threads unparked but performed no work before parking again.
The worker no-op count increases by one each time the worker unparks the thread but finds no new work and goes back to sleep. This indicates a false-positive wake up."#,
);
const MAX_NOOP_COUNT: (&str, &str) = (
    "rust_tokio_noops_max",
    r#"The maximum number of times any worker thread unparked but performed no work before parking again."#,
);
const MIN_NOOP_COUNT: (&str, &str) = (
    "rust_tokio_noops_min",
    r#"The minimum number of times any worker thread unparked but performed no work before parking again."#,
);
const TOTAL_STEAL_COUNT: (&str, &str) = (
    "rust_tokio_steals_total",
    r#"The number of tasks worker threads stole from another worker thread.
The worker steal count increases by the amount of stolen tasks each time the worker has processed its scheduled queue and successfully steals more pending tasks from another worker.
This metric only applies to the multi-threaded runtime and will always return 0 when using the current thread runtime."#,
);
const MAX_STEAL_COUNT: (&str, &str) = (
    "rust_tokio_steals_max",
    r#"The maximum number of tasks any worker thread stole from another worker thread."#,
);
const MIN_STEAL_COUNT: (&str, &str) = (
    "rust_tokio_steals_min",
    r#"The minimum number of tasks any worker thread stole from another worker thread."#,
);
const TOTAL_STEAL_OPERATIONS: (&str, &str) = (
    "rust_tokio_steal_operations_total",
    r#"The number of times worker threads stole tasks from another worker thread.
The worker steal operations increases by one each time the worker has processed its scheduled queue and successfully steals more pending tasks from another worker.
This metric only applies to the multi-threaded runtime and will always return 0 when using the current thread runtime."#,
);
const MAX_STEAL_OPERATIONS: (&str, &str) = (
    "rust_tokio_steal_operations_max",
    r#"The maximum number of times any worker thread stole tasks from another worker thread."#,
);
const MIN_STEAL_OPERATIONS: (&str, &str) = (
    "rust_tokio_steal_operations_min",
    r#"The minimum number of times any worker thread stole tasks from another worker thread."#,
);
const REMOTE_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_remote_schedules_total",
    r#"The number of tasks scheduled from outside of the runtime.
The remote schedule count increases by one each time a task is woken from outside of the runtime. This usually means that a task is spawned or notified from a non-runtime thread and must be queued using the Runtime’s injection queue, which tends to be slower."#,
);
const TOTAL_LOCAL_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_local_schedules_total",
    r#"The number of tasks scheduled from worker threads.
The local schedule count increases by one each time a task is woken from inside of the runtime. This usually means that a task is spawned or notified from within a runtime thread and will be queued on the worker-local queue."#,
);
const MAX_LOCAL_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_local_schedules_max",
    r#"The maximum number of tasks scheduled from any one worker thread."#,
);
const MIN_LOCAL_SCHEDULE_COUNT: (&str, &str) = (
    "rust_tokio_local_schedules_min",
    r#"The minimum number of tasks scheduled from any one worker thread."#,
);
const TOTAL_OVERFLOW_COUNT: (&str, &str) = (
    "rust_tokio_overflows_total",
    r#"The number of times worker threads saturated their local queues.
The worker steal count increases by one each time the worker attempts to schedule a task locally, but its local queue is full. When this happens, half of the local queue is moved to the injection queue.
This metric only applies to the multi-threaded scheduler."#,
);
const MAX_OVERFLOW_COUNT: (&str, &str) = (
    "rust_tokio_overflows_max",
    r#"The maximum number of times any one worker saturated its local queue."#,
);
const MIN_OVERFLOW_COUNT: (&str, &str) = (
    "rust_tokio_overflows_min",
    r#"The minimum number of times any one worker saturated its local queue."#,
);
const TOTAL_POLLS_COUNT: (&str, &str) = (
    "rust_tokio_polls_total",
    r#"The number of tasks that have been polled across all worker threads.
The worker poll count increases by one each time a worker polls a scheduled task."#,
);
const MAX_POLLS_COUNT: (&str, &str) = (
    "rust_tokio_polls_max",
    r#"The maximum number of tasks that have been polled in any worker thread."#,
);
const MIN_POLLS_COUNT: (&str, &str) = (
    "rust_tokio_polls_min",
    r#"The minimum number of tasks that have been polled in any worker thread."#,
);
const TOTAL_BUSY_DURATION_SECONDS: (&str, &str) = (
    "rust_tokio_busy_duration_seconds",
    r#"The amount of time worker threads were busy.
The worker busy duration increases whenever the worker is spending time processing work. Using this value can indicate the total load of workers."#,
);
const MAX_BUSY_DURATION_SECONDS: (&str, &str) = (
    "rust_tokio_busy_duration_seconds_max",
    r#"The maximum amount of time a worker thread was busy."#,
);
const MIN_BUSY_DURATION_SECONDS: (&str, &str) = (
    "rust_tokio_busy_duration_seconds_min",
    r#"The minimum amount of time a worker thread was busy."#,
);
const INJECTION_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_injection_queue_depth_gauge",
    r#"The number of tasks currently scheduled in the runtime’s injection queue.
Tasks that are spawned or notified from a non-runtime thread are scheduled using the runtime’s injection queue. This metric returns the current number of tasks pending in the injection queue. As such, the returned value may increase or decrease as new tasks are scheduled and processed."#,
);
const TOTAL_LOCAL_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_local_queue_depth_gauge_total",
    r#"The total number of tasks currently scheduled in workers’ local queues.
Tasks that are spawned or notified from within a runtime thread are scheduled using that worker’s local queue. This metric returns the current number of tasks pending in all workers’ local queues. As such, the returned value may increase or decrease as new tasks are scheduled and processed."#,
);
const MAX_LOCAL_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_local_queue_depth_gauge_max",
    r#"The maximum number of tasks currently scheduled any worker’s local queue."#,
);
const MIN_LOCAL_QUEUE_DEPTH_GAUGE: (&str, &str) = (
    "rust_tokio_local_queue_depth_gauge_min",
    r#"The minimum number of tasks currently scheduled any worker’s local queue."#,
);
const BUDGET_FORCED_YIELD_COUNT: (&str, &str) = (
    "rust_tokio_budget_forced_yield_count",
    r#"The number of times that tasks have been forced to yield back to the scheduler after exhausting their task budgets.
This count starts at zero when the runtime is created and increases by one each time a task yields due to exhausting its budget.
The counter is monotonically increasing. It is never decremented or reset to zero."#,
);
const IO_DRIVER_READY_COUNT: (&str, &str) = (
    "rust_tokio_io_driver_ready_count",
    r#"The number of ready events processed by the runtime’s I/O driver."#,
);
const MEAN_POLLS_PER_PARK: (&str, &str) = (
    "rust_tokio_mean_polls_per_park",
    r#"Mean number of tasks that have been polled per worker park event."#,
);
const BUSY_RATIO: (&str, &str) = (
    "rust_tokio_busy_ratio",
    formatcp!(
        r#"Ratio of load over the runtime workers during the scrape interval.
Ideally this ratio should be close to the number of workers in the runtime ({})"#,
        WORKERS_COUNT.0
    ),
);

/// A collector which exports the current state of tokio runtime metrics.
#[derive(Debug)]
pub struct TokioCollector {
    // "sum", "min", and "max" in field names mean "across tokio workers in the runtime"
    metrics_iter: Arc<Mutex<RuntimeIntervals>>,
    descs: Vec<Desc>,
    workers: IntGauge,
    sum_parks: IntCounter,
    min_parks: IntCounter,
    max_parks: IntCounter,
    sum_noops: IntCounter,
    max_noops: IntCounter,
    min_noops: IntCounter,
    sum_stolen_tasks: IntCounter,
    max_stolen_tasks: IntCounter,
    min_stolen_tasks: IntCounter,
    sum_steals: IntCounter,
    max_steals: IntCounter,
    min_steals: IntCounter,
    remote_schedules: IntCounter,
    sum_local_schedules: IntCounter,
    max_local_schedules: IntCounter,
    min_local_schedules: IntCounter,
    sum_overflows: IntCounter,
    max_overflows: IntCounter,
    min_overflows: IntCounter,
    sum_polls: IntCounter,
    max_polls: IntCounter,
    min_polls: IntCounter,
    sum_busy_duration: Counter,
    max_busy_duration: Counter,
    min_busy_duration: Counter,
    injection_queue_depth: IntGauge,
    sum_local_queue_depth: IntGauge,
    min_local_queue_depth: IntGauge,
    max_local_queue_depth: IntGauge,
    budget_forced_yield_tasks: IntCounter,
    io_driver_ready_events: IntCounter,
    mean_polls_per_park: Gauge,
    busy_ratio: Gauge,
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

        let sum_parks = IntCounter::with_opts(
            Opts::new(TOTAL_PARK_COUNT.0, TOTAL_PARK_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_parks.desc().into_iter().cloned());

        let min_parks = IntCounter::with_opts(
            Opts::new(MIN_PARK_COUNT.0, MIN_PARK_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_parks.desc().into_iter().cloned());

        let max_parks = IntCounter::with_opts(
            Opts::new(MAX_PARK_COUNT.0, MAX_PARK_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_parks.desc().into_iter().cloned());

        let sum_noops = IntCounter::with_opts(
            Opts::new(TOTAL_NOOP_COUNT.0, TOTAL_NOOP_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_noops.desc().into_iter().cloned());

        let max_noops = IntCounter::with_opts(
            Opts::new(MAX_NOOP_COUNT.0, MAX_NOOP_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_noops.desc().into_iter().cloned());

        let min_noops = IntCounter::with_opts(
            Opts::new(MIN_NOOP_COUNT.0, MIN_NOOP_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_noops.desc().into_iter().cloned());

        let sum_stolen_tasks = IntCounter::with_opts(
            Opts::new(TOTAL_STEAL_COUNT.0, TOTAL_STEAL_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_stolen_tasks.desc().into_iter().cloned());

        let max_stolen_tasks = IntCounter::with_opts(
            Opts::new(MAX_STEAL_COUNT.0, MAX_STEAL_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_stolen_tasks.desc().into_iter().cloned());

        let min_stolen_tasks = IntCounter::with_opts(
            Opts::new(MIN_STEAL_COUNT.0, MIN_STEAL_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_stolen_tasks.desc().into_iter().cloned());

        let sum_steals = IntCounter::with_opts(
            Opts::new(TOTAL_STEAL_OPERATIONS.0, TOTAL_STEAL_OPERATIONS.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_steals.desc().into_iter().cloned());

        let max_steals = IntCounter::with_opts(
            Opts::new(MAX_STEAL_OPERATIONS.0, MAX_STEAL_OPERATIONS.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_steals.desc().into_iter().cloned());

        let min_steals = IntCounter::with_opts(
            Opts::new(MIN_STEAL_OPERATIONS.0, MIN_STEAL_OPERATIONS.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_steals.desc().into_iter().cloned());

        let remote_schedules = IntCounter::with_opts(
            Opts::new(REMOTE_SCHEDULE_COUNT.0, REMOTE_SCHEDULE_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(remote_schedules.desc().into_iter().cloned());

        let sum_local_schedules = IntCounter::with_opts(
            Opts::new(TOTAL_LOCAL_SCHEDULE_COUNT.0, TOTAL_LOCAL_SCHEDULE_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_local_schedules.desc().into_iter().cloned());

        let max_local_schedules = IntCounter::with_opts(
            Opts::new(MAX_LOCAL_SCHEDULE_COUNT.0, MAX_LOCAL_SCHEDULE_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_local_schedules.desc().into_iter().cloned());

        let min_local_schedules = IntCounter::with_opts(
            Opts::new(MIN_LOCAL_SCHEDULE_COUNT.0, MIN_LOCAL_SCHEDULE_COUNT.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_local_schedules.desc().into_iter().cloned());

        let sum_overflows = IntCounter::with_opts(
            Opts::new(TOTAL_OVERFLOW_COUNT.0, TOTAL_OVERFLOW_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_overflows.desc().into_iter().cloned());

        let max_overflows = IntCounter::with_opts(
            Opts::new(MAX_OVERFLOW_COUNT.0, MAX_OVERFLOW_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_overflows.desc().into_iter().cloned());

        let min_overflows = IntCounter::with_opts(
            Opts::new(MIN_OVERFLOW_COUNT.0, MIN_OVERFLOW_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_overflows.desc().into_iter().cloned());

        let sum_polls = IntCounter::with_opts(
            Opts::new(TOTAL_POLLS_COUNT.0, TOTAL_POLLS_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_polls.desc().into_iter().cloned());

        let min_polls = IntCounter::with_opts(
            Opts::new(MIN_POLLS_COUNT.0, MIN_POLLS_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_polls.desc().into_iter().cloned());

        let max_polls = IntCounter::with_opts(
            Opts::new(MAX_POLLS_COUNT.0, MAX_POLLS_COUNT.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_polls.desc().into_iter().cloned());

        let sum_busy_duration = Counter::with_opts(
            Opts::new(TOTAL_BUSY_DURATION_SECONDS.0, TOTAL_BUSY_DURATION_SECONDS.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_busy_duration.desc().into_iter().cloned());

        let max_busy_duration = Counter::with_opts(
            Opts::new(MAX_BUSY_DURATION_SECONDS.0, MAX_BUSY_DURATION_SECONDS.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_busy_duration.desc().into_iter().cloned());

        let min_busy_duration = Counter::with_opts(
            Opts::new(MIN_BUSY_DURATION_SECONDS.0, MIN_BUSY_DURATION_SECONDS.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_busy_duration.desc().into_iter().cloned());

        let injection_queue_depth = IntGauge::with_opts(
            Opts::new(INJECTION_QUEUE_DEPTH_GAUGE.0, INJECTION_QUEUE_DEPTH_GAUGE.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(injection_queue_depth.desc().into_iter().cloned());

        let sum_local_queue_depth = IntGauge::with_opts(
            Opts::new(
                TOTAL_LOCAL_QUEUE_DEPTH_GAUGE.0,
                TOTAL_LOCAL_QUEUE_DEPTH_GAUGE.1,
            )
            .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(sum_local_queue_depth.desc().into_iter().cloned());

        let max_local_queue_depth = IntGauge::with_opts(
            Opts::new(MAX_LOCAL_QUEUE_DEPTH_GAUGE.0, MAX_LOCAL_QUEUE_DEPTH_GAUGE.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(max_local_queue_depth.desc().into_iter().cloned());

        let min_local_queue_depth = IntGauge::with_opts(
            Opts::new(MIN_LOCAL_QUEUE_DEPTH_GAUGE.0, MIN_LOCAL_QUEUE_DEPTH_GAUGE.1)
                .namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(min_local_queue_depth.desc().into_iter().cloned());

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
            Opts::new(MEAN_POLLS_PER_PARK.0, MEAN_POLLS_PER_PARK.1).namespace(namespace.clone()),
        )
        .unwrap();
        descs.extend(mean_polls_per_park.desc().into_iter().cloned());

        let busy_ratio =
            Gauge::with_opts(Opts::new(BUSY_RATIO.0, BUSY_RATIO.1).namespace(namespace)).unwrap();
        descs.extend(busy_ratio.desc().into_iter().cloned());

        let monitor = RuntimeMonitor::new(handle);
        let metrics_iter = Arc::new(Mutex::new(monitor.intervals()));

        Self {
            metrics_iter,
            descs,
            workers,
            sum_parks,
            min_parks,
            max_parks,
            sum_noops,
            max_noops,
            min_noops,
            sum_stolen_tasks,
            max_stolen_tasks,
            min_stolen_tasks,
            sum_steals,
            max_steals,
            min_steals,
            remote_schedules,
            sum_local_schedules,
            max_local_schedules,
            min_local_schedules,
            sum_overflows,
            max_overflows,
            min_overflows,
            sum_polls,
            max_polls,
            min_polls,
            sum_busy_duration,
            max_busy_duration,
            min_busy_duration,
            injection_queue_depth,
            sum_local_queue_depth,
            min_local_queue_depth,
            max_local_queue_depth,
            budget_forced_yield_tasks,
            io_driver_ready_events,
            mean_polls_per_park,
            busy_ratio,
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
        // TODO: no panic here.
        let metrics = self.metrics_iter.lock().unwrap().next().unwrap();

        // collect MetricFamilys.
        let mut mfs = Vec::with_capacity(METRICS_NUMBER);

        tracing::trace!("Collect numbers");
        {
            let count = metrics.workers_count;
            self.workers.set(count as i64);
            mfs.extend(self.workers.collect());
        }

        {
            let delta = metrics.total_park_count;
            self.sum_parks.inc_by(delta);
            mfs.extend(self.sum_parks.collect());
        }

        {
            let delta = metrics.max_park_count;
            self.max_parks.inc_by(delta);
            mfs.extend(self.max_parks.collect());
        }

        {
            let delta = metrics.min_park_count;
            self.min_parks.inc_by(delta);
            mfs.extend(self.min_parks.collect());
        }

        {
            let delta = metrics.total_noop_count;
            self.sum_noops.inc_by(delta);
            mfs.extend(self.sum_noops.collect());
        }

        {
            let delta = metrics.max_noop_count;
            self.max_noops.inc_by(delta);
            mfs.extend(self.max_noops.collect());
        }

        {
            let delta = metrics.min_noop_count;
            self.min_noops.inc_by(delta);
            mfs.extend(self.min_noops.collect());
        }

        {
            let delta = metrics.total_steal_count;
            self.sum_stolen_tasks.inc_by(delta);
            mfs.extend(self.sum_stolen_tasks.collect());
        }

        {
            let delta = metrics.max_steal_count;
            self.max_stolen_tasks.inc_by(delta);
            mfs.extend(self.max_stolen_tasks.collect());
        }

        {
            let delta = metrics.min_steal_count;
            self.min_stolen_tasks.inc_by(delta);
            mfs.extend(self.min_stolen_tasks.collect());
        }

        {
            let delta = metrics.total_steal_operations;
            self.sum_steals.inc_by(delta);
            mfs.extend(self.sum_steals.collect());
        }

        {
            let delta = metrics.max_steal_operations;
            self.max_steals.inc_by(delta);
            mfs.extend(self.max_steals.collect());
        }

        {
            let delta = metrics.min_steal_operations;
            self.min_steals.inc_by(delta);
            mfs.extend(self.min_steals.collect());
        }

        {
            let past = self.remote_schedules.get();
            let new = metrics.num_remote_schedules;
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.remote_schedules.inc_by(new.saturating_sub(past));
            mfs.extend(self.remote_schedules.collect());
        }

        {
            let delta = metrics.total_local_schedule_count;
            self.sum_local_schedules.inc_by(delta);
            mfs.extend(self.sum_local_schedules.collect());
        }

        {
            let delta = metrics.max_local_schedule_count;
            self.max_local_schedules.inc_by(delta);
            mfs.extend(self.max_local_schedules.collect());
        }

        {
            let delta = metrics.min_local_schedule_count;
            self.min_local_schedules.inc_by(delta);
            mfs.extend(self.min_local_schedules.collect());
        }

        {
            let delta = metrics.total_overflow_count;
            self.sum_overflows.inc_by(delta);
            mfs.extend(self.sum_overflows.collect());
        }

        {
            let delta = metrics.max_overflow_count;
            self.max_overflows.inc_by(delta);
            mfs.extend(self.max_overflows.collect());
        }

        {
            let delta = metrics.min_overflow_count;
            self.min_overflows.inc_by(delta);
            mfs.extend(self.min_overflows.collect());
        }

        {
            let delta = metrics.total_polls_count;
            self.sum_polls.inc_by(delta);
            mfs.extend(self.sum_polls.collect());
        }

        {
            let delta = metrics.max_polls_count;
            self.max_polls.inc_by(delta);
            mfs.extend(self.max_polls.collect());
        }

        {
            let delta = metrics.min_polls_count;
            self.min_polls.inc_by(delta);
            mfs.extend(self.min_polls.collect());
        }

        {
            let delta = metrics.total_busy_duration.as_secs_f64();
            self.sum_busy_duration.inc_by(delta);
            mfs.extend(self.sum_busy_duration.collect());
        }

        {
            let delta = metrics.max_busy_duration.as_secs_f64();
            self.max_busy_duration.inc_by(delta);
            mfs.extend(self.max_busy_duration.collect());
        }

        {
            let delta = metrics.min_busy_duration.as_secs_f64();
            self.min_busy_duration.inc_by(delta);
            mfs.extend(self.min_busy_duration.collect());
        }

        {
            let past = self.budget_forced_yield_tasks.get();
            let new = metrics.budget_forced_yield_count;
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.budget_forced_yield_tasks
                .inc_by(new.saturating_sub(past));
            mfs.extend(self.budget_forced_yield_tasks.collect());
        }

        {
            let new = metrics.injection_queue_depth as i64;
            self.injection_queue_depth.set(new);
            mfs.extend(self.injection_queue_depth.collect())
        }

        {
            let new = metrics.total_local_queue_depth as i64;
            self.sum_local_queue_depth.set(new);
            mfs.extend(self.sum_local_queue_depth.collect())
        }

        {
            let new = metrics.max_local_queue_depth as i64;
            self.max_local_queue_depth.set(new);
            mfs.extend(self.max_local_queue_depth.collect())
        }

        {
            let new = metrics.min_local_queue_depth as i64;
            self.min_local_queue_depth.set(new);
            mfs.extend(self.min_local_queue_depth.collect())
        }

        {
            let past = self.io_driver_ready_events.get();
            let new = metrics.io_driver_ready_count;
            debug_assert!(new >= past, "new: {new} >= past: {past}");
            self.io_driver_ready_events.inc_by(new.saturating_sub(past));
            mfs.extend(self.io_driver_ready_events.collect());
        }

        {
            let new = metrics.mean_polls_per_park();
            self.mean_polls_per_park.set(new);
            mfs.extend(self.mean_polls_per_park.collect())
        }

        {
            let new = metrics.busy_ratio();
            self.busy_ratio.set(new);
            mfs.extend(self.busy_ratio.collect())
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
            // Seven metrics per process collector.
            let descs = tc.desc();
            assert_eq!(descs.len(), super::METRICS_NUMBER);
            let mfs = tc.collect();
            assert_eq!(mfs.len(), super::METRICS_NUMBER);
        }

        let r = Registry::new();
        let res = r.register(Box::new(tc));
        assert!(res.is_ok());
    }
}
