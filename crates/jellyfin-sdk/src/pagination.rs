//! Pagination helpers.
//!
//! Jellyfin frequently uses `startIndex` + `limit` query parameters and returns results in a
//! `QueryResult<T>`-shaped payload. This module provides a small, ergonomic pager on top of that.

use std::marker::PhantomData;

use reqwest::Method;

use crate::{JellyfinClient, Result, models::QueryResult};

/// A pager for endpoints that use `startIndex`/`limit` and return `QueryResult<T>`.
#[derive(Clone, Debug)]
pub struct QueryPager<T> {
    client: JellyfinClient,
    method: Method,
    path: String,
    base_query: Vec<(String, String)>,
    limit: u32,
    next_start_index: u32,
    done: bool,
    _marker: PhantomData<T>,
}

impl<T> QueryPager<T> {
    /// Creates a pager.
    pub fn new(
        client: JellyfinClient,
        method: Method,
        path: impl Into<String>,
        base_query: Vec<(String, String)>,
    ) -> Self {
        Self {
            client,
            method,
            path: path.into(),
            base_query,
            limit: 100,
            next_start_index: 0,
            done: false,
            _marker: PhantomData,
        }
    }

    /// Sets the page size (`limit` query parameter).
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// Sets the first page start index (`startIndex` query parameter).
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.next_start_index = start_index;
        self
    }
}

impl<T: serde::de::DeserializeOwned> QueryPager<T> {
    /// Fetches the next page.
    ///
    /// Returns `Ok(None)` when no more data is available.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn next_page(&mut self) -> Result<Option<QueryResult<T>>> {
        if self.done {
            return Ok(None);
        }

        let mut query = self.base_query.clone();
        query.push(("startIndex".to_owned(), self.next_start_index.to_string()));
        query.push(("limit".to_owned(), self.limit.to_string()));

        let req = self
            .client
            .request(self.method.clone(), &self.path)?
            .query(&query);

        let page: QueryResult<T> = self.client.send_json(req).await?;

        let received = page.items.len() as u32;
        if received == 0 {
            self.done = true;
            return Ok(None);
        }

        self.next_start_index = self.next_start_index.saturating_add(received);

        if let Some(total) = page.total_record_count
            && self.next_start_index >= total
        {
            self.done = true;
        }

        Ok(Some(page))
    }
}
