use serde::{Deserialize, Serialize};

/// Task state options.
///
/// OpenAPI: `TaskState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TaskState {
    /// The task is not running.
    Idle,
    /// The task is in the process of cancelling.
    Cancelling,
    /// The task is running.
    Running,
}

/// Task completion status.
///
/// OpenAPI: `TaskCompletionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TaskCompletionStatus {
    /// Completed successfully.
    Completed,
    /// Completed with an error.
    Failed,
    /// Cancelled.
    Cancelled,
    /// Aborted.
    Aborted,
}

/// Task trigger type.
///
/// OpenAPI: `TaskTriggerInfoType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TaskTriggerInfoType {
    /// Runs daily at a configured time.
    DailyTrigger,
    /// Runs weekly at a configured day/time.
    WeeklyTrigger,
    /// Runs on an interval.
    IntervalTrigger,
    /// Runs at startup.
    StartupTrigger,
}

/// Day of week.
///
/// OpenAPI: `DayOfWeek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DayOfWeek {
    /// Sunday.
    Sunday,
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
}

/// Task execution details.
///
/// OpenAPI: `TaskResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskResult {
    /// Start time in UTC (ISO 8601).
    pub start_time_utc: Option<String>,
    /// End time in UTC (ISO 8601).
    pub end_time_utc: Option<String>,
    /// Completion status.
    pub status: Option<TaskCompletionStatus>,
    /// Name.
    pub name: Option<String>,
    /// Key.
    pub key: Option<String>,
    /// Id.
    pub id: Option<String>,
    /// Error message.
    pub error_message: Option<String>,
    /// Long error message.
    pub long_error_message: Option<String>,
}

/// Task trigger info.
///
/// OpenAPI: `TaskTriggerInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskTriggerInfo {
    /// Trigger type.
    #[serde(rename = "Type")]
    pub kind: Option<TaskTriggerInfoType>,
    /// Time of day ticks.
    pub time_of_day_ticks: Option<i64>,
    /// Interval ticks.
    pub interval_ticks: Option<i64>,
    /// Day of week.
    pub day_of_week: Option<DayOfWeek>,
    /// Maximum runtime ticks.
    pub max_runtime_ticks: Option<i64>,
}

/// A scheduled task.
///
/// OpenAPI: `TaskInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskInfo {
    /// Name.
    pub name: Option<String>,
    /// State.
    pub state: Option<TaskState>,
    /// Current progress percentage.
    pub current_progress_percentage: Option<f64>,
    /// Task id.
    pub id: Option<String>,
    /// Last execution result.
    pub last_execution_result: Option<TaskResult>,
    /// Triggers.
    pub triggers: Option<Vec<TaskTriggerInfo>>,
    /// Description.
    pub description: Option<String>,
    /// Category.
    pub category: Option<String>,
    /// Whether this task is hidden.
    pub is_hidden: Option<bool>,
    /// Key.
    pub key: Option<String>,
}
