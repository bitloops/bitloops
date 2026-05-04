use async_graphql::{Context, Result};

use crate::capability_packs::navigation_context::storage::{
    accept_navigation_context_view, materialise_navigation_context_view,
};
use crate::graphql::DevqlGraphqlContext;
use crate::graphql::types::{
    AcceptNavigationContextViewInput, AcceptNavigationContextViewResult,
    MaterialiseNavigationContextViewInput, MaterialiseNavigationContextViewResult,
};

use super::errors::operation_error;

pub(super) async fn accept_navigation_context_view_signature(
    ctx: &Context<'_>,
    input: AcceptNavigationContextViewInput,
) -> Result<AcceptNavigationContextViewResult> {
    let operation = "acceptNavigationContextView";
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err))?;
    let view_id = require_non_empty(input.view_id, "viewId", operation)?;
    let relational = context
        .open_relational_storage(operation)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", operation, err))?;
    let materialised_ref = match input.materialised_ref.as_deref() {
        Some(materialised_ref) if !materialised_ref.trim().is_empty() => {
            Some(materialised_ref.trim().to_string())
        }
        _ => {
            let materialisation = materialise_navigation_context_view(
                &relational,
                context.repo_id(),
                &view_id,
                input.expected_current_signature.as_deref(),
            )
            .await
            .map_err(|err| operation_error("BACKEND_ERROR", "storage", operation, err))?
            .ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    format!("unknown navigation context view `{view_id}`"),
                )
            })?;
            Some(materialisation.materialised_ref)
        }
    };
    let acceptance = accept_navigation_context_view(
        &relational,
        context.repo_id(),
        &view_id,
        input.expected_current_signature.as_deref(),
        input.source.as_deref(),
        input.reason.as_deref(),
        materialised_ref.as_deref(),
    )
    .await
    .map_err(|err| operation_error("BACKEND_ERROR", "storage", operation, err))?
    .ok_or_else(|| {
        operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("unknown navigation context view `{view_id}`"),
        )
    })?;

    Ok(AcceptNavigationContextViewResult::from_acceptance(
        acceptance,
    ))
}

pub(super) async fn materialise_navigation_context_view_snapshot(
    ctx: &Context<'_>,
    input: MaterialiseNavigationContextViewInput,
) -> Result<MaterialiseNavigationContextViewResult> {
    let operation = "materialiseNavigationContextView";
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err))?;
    let view_id = require_non_empty(input.view_id, "viewId", operation)?;
    let relational = context
        .open_relational_storage(operation)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", operation, err))?;
    let materialisation = materialise_navigation_context_view(
        &relational,
        context.repo_id(),
        &view_id,
        input.expected_current_signature.as_deref(),
    )
    .await
    .map_err(|err| operation_error("BACKEND_ERROR", "storage", operation, err))?
    .ok_or_else(|| {
        operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("unknown navigation context view `{view_id}`"),
        )
    })?;

    Ok(MaterialiseNavigationContextViewResult::from_materialisation(materialisation))
}

fn require_non_empty(
    value: String,
    field: &'static str,
    operation: &'static str,
) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("`{field}` must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}
