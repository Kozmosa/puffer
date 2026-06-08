use super::{block_on_manager_handle, SubscriptionManager, CONNECTOR_COMMAND_TIMEOUT};
use crate::connector_process;
use crate::contact_history;
use crate::contacts::{
    connector_contacts_for_connector, contact_ids_for_connector, ConnectorContact, ContactContext,
};
use anyhow::Result;

impl SubscriptionManager {
    /// Runs a connector template's contact-list command or history fallback.
    pub fn list_connector_contacts(
        &self,
        connection_slug: &str,
        query: Option<String>,
        limit: Option<usize>,
    ) -> Result<Option<Vec<ConnectorContact>>> {
        let (template, connection_slug) = self.template_for_connection(connection_slug)?;
        let connector_slug = template.slug.clone();
        let process_connection_slug = connection_slug.clone();
        let query_for_command = query.clone();
        let contacts = block_on_manager_handle(&self.handle, async move {
            connector_process::run_list_contacts(
                &template,
                &process_connection_slug,
                query_for_command,
                limit,
                CONNECTOR_COMMAND_TIMEOUT,
            )
            .await
        })?;
        Ok(Some(match contacts {
            Some(contacts) => connector_contacts_for_connector(&connector_slug, contacts),
            None => contact_history::list_contacts(
                &self.history_store,
                &connector_slug,
                &connection_slug,
                query.as_deref(),
                limit,
            ),
        }))
    }

    /// Runs a connector template's contact-search command or history fallback.
    pub fn search_connector_contacts(
        &self,
        connection_slug: &str,
        query: String,
        limit: Option<usize>,
    ) -> Result<Option<Vec<ConnectorContact>>> {
        let (template, connection_slug) = self.template_for_connection(connection_slug)?;
        let connector_slug = template.slug.clone();
        let process_connection_slug = connection_slug.clone();
        let query_for_command = query.clone();
        let contacts = block_on_manager_handle(&self.handle, async move {
            connector_process::run_search_contacts(
                &template,
                &process_connection_slug,
                query_for_command,
                limit,
                CONNECTOR_COMMAND_TIMEOUT,
            )
            .await
        })?;
        Ok(Some(match contacts {
            Some(contacts) => connector_contacts_for_connector(&connector_slug, contacts),
            None => contact_history::list_contacts(
                &self.history_store,
                &connector_slug,
                &connection_slug,
                Some(&query),
                limit,
            ),
        }))
    }

    /// Runs a connector template's contact-context command or history fallback.
    pub fn connector_contact_context(
        &self,
        connection_slug: &str,
        contact_ids: Vec<String>,
        limit: Option<usize>,
    ) -> Result<Option<(Vec<String>, Vec<ContactContext>)>> {
        let connection = self
            .connection_store
            .get(connection_slug)
            .ok_or_else(|| anyhow::anyhow!("connection `{connection_slug}` not found"))?;
        let contact_ids = contact_ids_for_connector(&connection.connector_slug, &contact_ids);
        if contact_ids.is_empty() {
            return Ok(None);
        }
        let (template, connection_slug) = self.template_for_connection(connection_slug)?;
        let process_connection_slug = connection_slug.clone();
        let contact_ids_for_command = contact_ids.clone();
        let context = block_on_manager_handle(&self.handle, async move {
            connector_process::run_contact_context(
                &template,
                &process_connection_slug,
                contact_ids_for_command,
                limit,
                CONNECTOR_COMMAND_TIMEOUT,
            )
            .await
        })?;
        Ok(context.or_else(|| {
            contact_history::contact_context(
                &self.history_store,
                &connection.connector_slug,
                &connection_slug,
                &contact_ids,
                limit,
            )
        }))
    }

    fn template_for_connection(
        &self,
        connection_slug: &str,
    ) -> Result<(crate::catalog::ConnectorTemplate, String)> {
        let connection = self
            .connection_store
            .get(connection_slug)
            .ok_or_else(|| anyhow::anyhow!("connection `{connection_slug}` not found"))?;
        let template = self
            .connector_store
            .get(&connection.connector_slug)
            .ok_or_else(|| {
                anyhow::anyhow!("connector `{}` not found", connection.connector_slug)
            })?;
        Ok((template, connection.slug))
    }
}
