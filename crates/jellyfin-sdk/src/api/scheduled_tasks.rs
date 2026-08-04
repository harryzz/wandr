use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{TaskInfo, TaskTriggerInfo},
};

/// Query parameters for `GET /ScheduledTasks`.
#[derive(Clone, Debug, Default)]
pub struct ScheduledTasksQuery {
    params: Vec<(String, String)>,
}

impl ScheduledTasksQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Optional filter tasks that are hidden, or not.
    pub fn is_hidden(mut self, is_hidden: bool) -> Self {
        self.params
            .push(("isHidden".to_owned(), is_hidden.to_string()));
        self
    }

    /// Optional filter tasks that are enabled, or not.
    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.params
            .push(("isEnabled".to_owned(), is_enabled.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn params(&self) -> &[(String, String)] {
        &self.params
    }
}

/// Scheduled task management endpoints.
#[derive(Clone, Debug)]
pub struct ScheduledTasksApi {
    client: JellyfinClient,
}

impl ScheduledTasksApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets scheduled tasks.
    ///
    /// OpenAPI: `GET /ScheduledTasks` (`GetTasks`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_tasks(&self, query: ScheduledTasksQuery) -> Result<Vec<TaskInfo>> {
        let mut req = self.client.request(Method::GET, "ScheduledTasks")?;
        if !query.params().is_empty() {
            req = req.query(query.params());
        }
        self.client.send_json(req).await
    }

    /// Gets a scheduled task by id.
    ///
    /// OpenAPI: `GET /ScheduledTasks/{taskId}` (`GetTask`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_task(&self, task_id: impl AsRef<str>) -> Result<TaskInfo> {
        let req = self
            .client
            .request(Method::GET, &format!("ScheduledTasks/{}", task_id.as_ref()))?;
        self.client.send_json(req).await
    }

    /// Starts a scheduled task.
    ///
    /// OpenAPI: `POST /ScheduledTasks/Running/{taskId}` (`StartTask`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn start_task(&self, task_id: impl AsRef<str>) -> Result<()> {
        let req = self.client.request(
            Method::POST,
            &format!("ScheduledTasks/Running/{}", task_id.as_ref()),
        )?;
        self.client.send_unit(req).await
    }

    /// Stops a scheduled task.
    ///
    /// OpenAPI: `DELETE /ScheduledTasks/Running/{taskId}` (`StopTask`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn stop_task(&self, task_id: impl AsRef<str>) -> Result<()> {
        let req = self.client.request(
            Method::DELETE,
            &format!("ScheduledTasks/Running/{}", task_id.as_ref()),
        )?;
        self.client.send_unit(req).await
    }

    /// Updates triggers for a task.
    ///
    /// OpenAPI: `POST /ScheduledTasks/{taskId}/Triggers` (`UpdateTask`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_task_triggers(
        &self,
        task_id: impl AsRef<str>,
        triggers: Vec<TaskTriggerInfo>,
    ) -> Result<()> {
        let req = self
            .client
            .request(
                Method::POST,
                &format!("ScheduledTasks/{}/Triggers", task_id.as_ref()),
            )?
            .json(&triggers);
        self.client.send_unit(req).await
    }
}
