use std::path::{Path, PathBuf};

use convert_case::{Case, Casing};
use handlebars::*;
use prepare::PrepareError;
use serde::{Deserialize, Serialize};

pub use codegen::{CodegenParams, CodegenResult};
pub use compile::CompileParams;
pub use parse::{ParseError, ParseParams};
pub use prepare::{PackageVersion, PrepareParams};
pub use test::TestParams;

mod codegen;
mod compile;
mod package_mgmt;
mod parse;
mod prepare;
mod test;
mod transpiler;

pub struct Builder<'a> {
    reg: Handlebars<'a>,
    entrypoint_service_main: Template<'a>,
    entrypoint_gateway_main: Template<'a>,
    entrypoint_combined_main: Template<'a>,

    catalog_clients_index_js: Template<'a>,
    catalog_clients_index_d_ts: Template<'a>,
    catalog_clients_service_js: Template<'a>,
    catalog_clients_service_testing_js: Template<'a>,
    catalog_clients_service_d_ts: Template<'a>,
    catalog_auth_index_ts: Template<'a>,
    catalog_auth_auth_ts: Template<'a>,
}

impl Builder<'_> {
    pub fn new() -> Result<Self, PrepareError> {
        let mut reg = Handlebars::new();
        reg.register_helper("toJSON", Box::new(to_json));
        reg.register_helper("stripExt", Box::new(strip_ext));
        reg.register_helper("toPascalCase", Box::new(to_pascal_case));
        reg.register_helper("encoreNameToIdent", Box::new(encore_name_to_ident));
        let entrypoint_service_main =
            Template::new(&mut reg, "service_main", ENTRYPOINT_SERVICE_MAIN)
                .map_err(|e| PrepareError::Internal(e.into()))?;
        let entrypoint_gateway_main =
            Template::new(&mut reg, "gateway_main", ENTRYPOINT_GATEWAY_MAIN)
                .map_err(|e| PrepareError::Internal(e.into()))?;
        let entrypoint_combined_main =
            Template::new(&mut reg, "combined_main", ENTRYPOINT_COMBINED_MAIN)
                .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_clients_index_js = Template::new(
            &mut reg,
            "catalog_clients_index_js",
            CATALOG_CLIENTS_INDEX_JS,
        )
        .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_clients_index_d_ts = Template::new(
            &mut reg,
            "catalog_clients_index_d_ts",
            CATALOG_CLIENTS_INDEX_D_TS,
        )
        .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_clients_service_js = Template::new(
            &mut reg,
            "catalog_clients_service_js",
            CATALOG_CLIENTS_SERVICE_JS,
        )
        .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_clients_service_testing_js = Template::new(
            &mut reg,
            "catalog_clients_service_test_js",
            CATALOG_CLIENTS_SERVICE_TESTING_JS,
        )
        .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_clients_service_d_ts = Template::new(
            &mut reg,
            "catalog_clients_service_d_ts",
            CATALOG_CLIENTS_SERVICE_D_TS,
        )
        .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_auth_index_ts =
            Template::new(&mut reg, "catalog_auth_index_ts", CATALOG_AUTH_INDEX_TS)
                .map_err(|e| PrepareError::Internal(e.into()))?;
        let catalog_auth_auth_ts =
            Template::new(&mut reg, "catalog_auth_auth_ts", CATALOG_AUTH_AUTH_TS)
                .map_err(|e| PrepareError::Internal(e.into()))?;

        Ok(Self {
            reg,
            entrypoint_service_main,
            entrypoint_gateway_main,
            entrypoint_combined_main,
            catalog_clients_index_js,
            catalog_clients_index_d_ts,
            catalog_clients_service_js,
            catalog_clients_service_testing_js,
            catalog_clients_service_d_ts,
            catalog_auth_index_ts,
            catalog_auth_auth_ts,
        })
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub root: PathBuf,
    pub platform_id: Option<String>,
    pub local_id: String,
}

impl App {
    /// Compute the relative path from the app root.
    /// It reports an error if the path is not under the app root.
    fn rel_path<'b>(&self, path: &'b Path) -> Result<&'b Path, PrepareError> {
        let suffix = path.strip_prefix(&self.root).map_err(|err| {
            PrepareError::Internal(anyhow::anyhow!(
                "unable to compute relative path to app root from {path:?}: {err}"
            ))
        })?;

        Ok(suffix)
    }

    /// Compute the relative path from the app root as a String.
    fn rel_path_string(&self, path: &Path) -> Result<String, PrepareError> {
        let suffix = self.rel_path(path)?;
        let s = suffix
            .to_str()
            .ok_or(PrepareError::Internal(anyhow::anyhow!(
                "invalid path: {:?}",
                path
            )))?;
        Ok(s.to_string())
    }
}

struct Template<'a> {
    name: &'a str,
}

impl<'a> Template<'a> {
    fn new(reg: &mut Handlebars, name: &'a str, template_str: &str) -> Result<Self, PrepareError> {
        reg.register_template_string(name, template_str)
            .map_err(|e| PrepareError::Internal(e.into()))?;

        Ok(Self { name })
    }

    fn render(&self, reg: &Handlebars, data: &impl Serialize) -> Result<String, PrepareError> {
        reg.render(self.name, data)
            .map_err(|e| PrepareError::Internal(e.into()))
    }
}

const ENTRYPOINT_SERVICE_MAIN: &str =
    include_str!("templates/entrypoints/services/main.handlebars");
const ENTRYPOINT_GATEWAY_MAIN: &str =
    include_str!("templates/entrypoints/gateways/main.handlebars");
const ENTRYPOINT_COMBINED_MAIN: &str =
    include_str!("templates/entrypoints/combined/main.handlebars");

const CATALOG_CLIENTS_INDEX_JS: &str =
    include_str!("templates/catalog/clients/index_js.handlebars");
const CATALOG_CLIENTS_INDEX_D_TS: &str =
    include_str!("templates/catalog/clients/index_d_ts.handlebars");
const CATALOG_CLIENTS_SERVICE_JS: &str =
    include_str!("templates/catalog/clients/endpoints_js.handlebars");
const CATALOG_CLIENTS_SERVICE_TESTING_JS: &str =
    include_str!("templates/catalog/clients/endpoints_testing_js.handlebars");
const CATALOG_CLIENTS_SERVICE_D_TS: &str =
    include_str!("templates/catalog/clients/endpoints_d_ts.handlebars");
const CATALOG_AUTH_INDEX_TS: &str = include_str!("templates/catalog/auth/index_ts.handlebars");
const CATALOG_AUTH_AUTH_TS: &str = include_str!("templates/catalog/auth/auth_ts.handlebars");

handlebars_helper!(strip_ext: |v: String| v.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(v));
handlebars_helper!(to_pascal_case: |v: String| v.to_case(Case::Pascal));
handlebars_helper!(encore_name_to_ident: |v: String| v.replace('-', "_"));

fn to_json(
    h: &Helper<'_, '_>,
    _: &Handlebars<'_>,
    _: &Context,
    _rc: &mut RenderContext<'_, '_>,
    out: &mut dyn Output,
) -> HelperResult {
    // get parameter from helper or throw an error
    let param = h
        .param(0)
        .map(|v| serde_json::to_string(v.value()).unwrap())
        .unwrap_or_default();
    out.write(param.as_ref())?;
    Ok(())
}

/// An error that is rendered plainly, without a backtrace.
#[derive(Debug)]
pub struct PlainError(pub String);

impl std::fmt::Display for PlainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Deserialize, Debug, Copy, Clone)]
#[serde(rename_all = "lowercase")]
pub enum DebugMode {
    Disabled,
    Enabled,
    Break,
}

#[derive(Deserialize, Debug, Copy, Clone)]
#[serde(rename_all = "lowercase")]
pub enum NodeJSRuntime {
    NodeJS,
    Bun,
}

impl Default for NodeJSRuntime {
    fn default() -> Self {
        Self::NodeJS
    }
}
