extern crate wasmcloud_control_interface;
use crate::ctx::{context_dir, get_default_context, load_context};
use crate::{
    ctl::manifest::HostManifest,
    util::{
        convert_error, labels_vec_to_hashmap, Output, OutputKind, Result, DEFAULT_LATTICE_PREFIX,
        DEFAULT_NATS_HOST, DEFAULT_NATS_PORT, DEFAULT_NATS_TIMEOUT,
    },
};
use id::{ModuleId, ServerId, ServiceId};
pub(crate) use output::*;
use spinners::{Spinner, Spinners};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use structopt::StructOpt;
use wasmcloud_control_interface::{
    Client as CtlClient, CtlOperationAck, GetClaimsResponse, Host, HostInventory,
    LinkDefinitionList,
};

mod id;
mod manifest;
mod output;

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct CtlCli {
    #[structopt(flatten)]
    command: CtlCliCommand,
}

impl CtlCli {
    pub(crate) fn command(self) -> CtlCliCommand {
        self.command
    }
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct ConnectionOpts {
    /// CTL Host for connection, defaults to 127.0.0.1 for local nats
    #[structopt(short = "r", long = "ctl-host", env = "WASMCLOUD_CTL_HOST")]
    ctl_host: Option<String>,

    /// CTL Port for connections, defaults to 4222 for local nats
    #[structopt(short = "p", long = "ctl-port", env = "WASMCLOUD_CTL_PORT")]
    ctl_port: Option<String>,

    /// JWT file for CTL authentication. Must be supplied with ctl_seed.
    #[structopt(long = "ctl-jwt", env = "WASMCLOUD_CTL_JWT", hide_env_values = true)]
    ctl_jwt: Option<String>,

    /// Seed file or literal for CTL authentication. Must be supplied with ctl_jwt.
    #[structopt(long = "ctl-seed", env = "WASMCLOUD_CTL_SEED", hide_env_values = true)]
    ctl_seed: Option<String>,

    /// Credsfile for CTL authentication. Combines ctl_seed and ctl_jwt.
    /// See https://docs.nats.io/developing-with-nats/security/creds for details.
    #[structopt(long = "ctl-credsfile", env = "WASH_CTL_CREDS", hide_env_values = true)]
    ctl_credsfile: Option<PathBuf>,

    /// Lattice prefix for wasmcloud control interface, defaults to "default"
    #[structopt(short = "x", long = "lattice-prefix", env = "WASMCLOUD_LATTICE_PREFIX")]
    lattice_prefix: Option<String>,

    /// Timeout length to await a control interface response, defaults to 2000 milliseconds
    #[structopt(short = "t", long = "timeout-ms", env = "WASMCLOUD_CTL_TIMEOUT_MS")]
    timeout_ms: Option<u64>,

    /// Path to a context with values to use for CTL connection and authentication
    #[structopt(long = "context")]
    pub(crate) context: Option<PathBuf>,
}

impl Default for ConnectionOpts {
    fn default() -> Self {
        ConnectionOpts {
            ctl_host: Some(DEFAULT_NATS_HOST.to_string()),
            ctl_port: Some(DEFAULT_NATS_PORT.to_string()),
            ctl_jwt: None,
            ctl_seed: None,
            ctl_credsfile: None,
            lattice_prefix: Some(DEFAULT_LATTICE_PREFIX.to_string()),
            timeout_ms: Some(DEFAULT_NATS_TIMEOUT),
            context: None,
        }
    }
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum CtlCliCommand {
    /// Retrieves information about the lattice
    #[structopt(name = "get")]
    Get(GetCommand),

    /// Link an actor and a provider
    #[structopt(name = "link")]
    Link(LinkCommand),

    /// Start an actor or a provider
    #[structopt(name = "start")]
    Start(StartCommand),

    /// Stop an actor, provider, or host
    #[structopt(name = "stop")]
    Stop(StopCommand),

    /// Update an actor running in a host to a new actor
    #[structopt(name = "update")]
    Update(UpdateCommand),

    /// Apply a manifest file to a target host
    #[structopt(name = "apply")]
    Apply(ApplyCommand),
}

#[derive(StructOpt, Debug, Clone)]
pub(crate) struct ApplyCommand {
    /// Public key of the target host for the manifest application
    #[structopt(name = "host-key", parse(try_from_str))]
    pub(crate) host_key: ServerId,

    /// Path to the manifest file. Note that all the entries in this file are imperative instructions, and all actor and provider references MUST be valid OCI references.
    #[structopt(name = "path")]
    pub(crate) path: String,

    /// Expand environment variables using substitution syntax within the manifest file
    #[structopt(name = "expand-env", short = "e", long = "expand-env")]
    pub(crate) expand_env: bool,

    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum GetCommand {
    /// Query lattice for running hosts
    #[structopt(name = "hosts")]
    Hosts(GetHostsCommand),

    /// Query a single host for its inventory of labels, actors and providers
    #[structopt(name = "inventory")]
    HostInventory(GetHostInventoryCommand),

    /// Query lattice for its claims cache
    #[structopt(name = "claims")]
    Claims(GetClaimsCommand),
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum LinkCommand {
    /// Query established links
    #[structopt(name = "query")]
    Query(LinkQueryCommand),

    /// Establish a link definition
    #[structopt(name = "put")]
    Put(LinkPutCommand),

    /// Delete a link definition
    #[structopt(name = "del")]
    Del(LinkDelCommand),
}

#[derive(StructOpt, Debug, Clone)]
pub(crate) struct LinkQueryCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,
}

#[derive(StructOpt, Debug, Clone)]
pub(crate) struct LinkDelCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Public key ID of actor
    #[structopt(name = "actor-id", parse(try_from_str))]
    pub(crate) actor_id: ModuleId,

    /// Capability contract ID between actor and provider
    #[structopt(name = "contract-id")]
    pub(crate) contract_id: String,

    /// Link name, defaults to "default"
    #[structopt(short = "l", long = "link-name")]
    pub(crate) link_name: Option<String>,
}

#[derive(StructOpt, Debug, Clone)]
pub(crate) struct LinkPutCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Public key ID of actor
    #[structopt(name = "actor-id", parse(try_from_str))]
    pub(crate) actor_id: ModuleId,

    /// Public key ID of provider
    #[structopt(name = "provider-id", parse(try_from_str))]
    pub(crate) provider_id: ServiceId,

    /// Capability contract ID between actor and provider
    #[structopt(name = "contract-id")]
    pub(crate) contract_id: String,

    /// Link name, defaults to "default"
    #[structopt(short = "l", long = "link-name")]
    pub(crate) link_name: Option<String>,

    /// Environment values to provide alongside link
    #[structopt(name = "values")]
    pub(crate) values: Vec<String>,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum StartCommand {
    /// Launch an actor in a host
    #[structopt(name = "actor")]
    Actor(StartActorCommand),

    /// Launch a provider in a host
    #[structopt(name = "provider")]
    Provider(StartProviderCommand),
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum StopCommand {
    /// Stop an actor running in a host
    #[structopt(name = "actor")]
    Actor(StopActorCommand),

    /// Stop a provider running in a host
    #[structopt(name = "provider")]
    Provider(StopProviderCommand),

    /// Purge and stop a running host
    #[structopt(name = "host")]
    Host(StopHostCommand),
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) enum UpdateCommand {
    /// Update an actor running in a host
    #[structopt(name = "actor")]
    Actor(UpdateActorCommand),
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct GetHostsCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct GetHostInventoryCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host
    #[structopt(name = "host-id", parse(try_from_str))]
    pub(crate) host_id: ServerId,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct GetClaimsCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct StartActorCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host, if omitted the actor will be auctioned in the lattice to find a suitable host
    #[structopt(short = "h", long = "host-id", name = "host-id", parse(try_from_str))]
    pub(crate) host_id: Option<ServerId>,

    /// Actor reference, e.g. the OCI URL for the actor.
    #[structopt(name = "actor-ref")]
    pub(crate) actor_ref: String,

    /// Constraints for actor auction in the form of "label=value". If host-id is supplied, this list is ignored
    #[structopt(short = "c", long = "constraint", name = "constraints")]
    constraints: Option<Vec<String>>,

    /// Timeout to await an auction response, defaults to 2000 milliseconds
    #[structopt(long = "auction-timeout-ms")]
    auction_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct StartProviderCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host, if omitted the provider will be auctioned in the lattice to find a suitable host
    #[structopt(short = "h", long = "host-id", name = "host-id", parse(try_from_str))]
    host_id: Option<ServerId>,

    /// Provider reference, e.g. the OCI URL for the provider
    #[structopt(name = "provider-ref")]
    pub(crate) provider_ref: String,

    /// Link name of provider
    #[structopt(short = "l", long = "link-name", default_value = "default")]
    pub(crate) link_name: String,

    /// Constraints for provider auction in the form of "label=value". If host-id is supplied, this list is ignored
    #[structopt(short = "c", long = "constraint", name = "constraints")]
    constraints: Option<Vec<String>>,

    /// Timeout to await an auction response, defaults to 2000 milliseconds
    #[structopt(long = "auction-timeout-ms")]
    auction_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct StopActorCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host
    #[structopt(name = "host-id", parse(try_from_str))]
    pub(crate) host_id: ServerId,

    /// Actor Id, e.g. the public key for the actor
    #[structopt(name = "actor-id", parse(try_from_str))]
    pub(crate) actor_id: ModuleId,

    /// Number of actors to stop
    #[structopt(long = "count", default_value = "1")]
    pub(crate) count: u16,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct StopProviderCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host
    #[structopt(name = "host-id", parse(try_from_str))]
    host_id: ServerId,

    /// Provider Id, e.g. the public key for the provider
    #[structopt(name = "provider-id", parse(try_from_str))]
    pub(crate) provider_id: ServiceId,

    /// Link name of provider
    #[structopt(name = "link-name")]
    pub(crate) link_name: String,

    /// Capability contract Id of provider
    #[structopt(name = "contract-id")]
    pub(crate) contract_id: String,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct StopHostCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host
    #[structopt(name = "host-id", parse(try_from_str))]
    host_id: ServerId,

    /// The timeout in ms for how much time to give the host for graceful shutdown
    #[structopt(short = "h", long = "host-timeout")]
    host_shutdown_timeout: Option<u64>,
}

#[derive(Debug, Clone, StructOpt)]
pub(crate) struct UpdateActorCommand {
    #[structopt(flatten)]
    opts: ConnectionOpts,

    #[structopt(flatten)]
    pub(crate) output: Output,

    /// Id of host
    #[structopt(name = "host-id", parse(try_from_str))]
    pub(crate) host_id: ServerId,

    /// Actor Id, e.g. the public key for the actor
    #[structopt(name = "actor-id", parse(try_from_str))]
    pub(crate) actor_id: ModuleId,

    /// Actor reference, e.g. the OCI URL for the actor.
    #[structopt(name = "new-actor-ref")]
    pub(crate) new_actor_ref: String,
}

pub(crate) async fn handle_command(command: CtlCliCommand) -> Result<String> {
    use CtlCliCommand::*;
    let mut sp: Option<Spinner> = None;
    let out = match command {
        Apply(cmd) => {
            let output = cmd.output;
            sp = update_spinner_message(sp, " Applying manifest ...".to_string(), &output);
            let results = apply_manifest(cmd).await?;
            apply_manifest_output(results, &output.kind)
        }
        Get(GetCommand::Hosts(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(sp, " Retrieving Hosts ...".to_string(), &output);
            let hosts = get_hosts(cmd).await?;
            get_hosts_output(hosts, &output.kind)
        }
        Get(GetCommand::HostInventory(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(
                sp,
                format!(" Retrieving inventory for host {} ...", cmd.host_id),
                &output,
            );
            let inv = get_host_inventory(cmd).await?;
            get_host_inventory_output(inv, &output.kind)
        }
        Get(GetCommand::Claims(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(sp, " Retrieving claims ... ".to_string(), &output);
            let claims = get_claims(cmd).await?;
            get_claims_output(claims, &output.kind)
        }
        Link(LinkCommand::Del(cmd)) => {
            let link_name = &cmd
                .link_name
                .clone()
                .unwrap_or_else(|| "default".to_string());
            sp = update_spinner_message(
                sp,
                format!(
                    "Deleting link for {} on {} ({}) ... ",
                    cmd.actor_id, cmd.contract_id, link_name,
                ),
                &cmd.output,
            );
            let failure = link_del(cmd.clone())
                .await
                .map_or_else(|e| Some(format!("{}", e)), |_| None);
            link_del_output(
                &cmd.actor_id,
                &cmd.contract_id,
                link_name,
                failure,
                &cmd.output.kind,
            )
        }
        Link(LinkCommand::Put(cmd)) => {
            sp = update_spinner_message(
                sp,
                format!(
                    "Defining link between {} and {} ... ",
                    cmd.actor_id, cmd.provider_id
                ),
                &cmd.output,
            );
            let failure = link_put(cmd.clone())
                .await
                .map_or_else(|e| Some(format!("{}", e)), |_| None);
            link_put_output(&cmd.actor_id, &cmd.provider_id, failure, &cmd.output.kind)
        }
        Link(LinkCommand::Query(cmd)) => {
            sp = update_spinner_message(sp, "Querying Links ... ".to_string(), &cmd.output);
            let result = link_query(cmd.clone()).await?;
            link_query_output(result, &cmd.output.kind)
        }
        Start(StartCommand::Actor(cmd)) => {
            let output = cmd.output;
            let actor_ref = &cmd.actor_ref.to_string();
            sp = update_spinner_message(sp, format!(" Starting actor {} ... ", actor_ref), &output);
            let ack = start_actor(cmd).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Actor {} started successfully", actor_ref),
                &ack.error,
                &output.kind,
            )
        }
        Start(StartCommand::Provider(cmd)) => {
            let output = cmd.output;
            let provider_ref = &cmd.provider_ref.to_string();
            sp = update_spinner_message(
                sp,
                format!(" Starting provider {} ... ", provider_ref),
                &output,
            );
            let ack = start_provider(cmd).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Provider {} started successfully", provider_ref),
                &ack.error,
                &output.kind,
            )
        }
        Stop(StopCommand::Actor(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(
                sp,
                format!(" Stopping actor {} ... ", cmd.actor_id),
                &output,
            );
            let ack = stop_actor(cmd.clone()).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Actor {} stopped successfully", cmd.actor_id),
                &ack.error,
                &output.kind,
            )
        }
        Stop(StopCommand::Provider(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(
                sp,
                format!(" Stopping provider {} ... ", cmd.provider_id),
                &output,
            );
            let ack = stop_provider(cmd.clone()).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Provider {} stopped successfully", cmd.provider_id),
                &ack.error,
                &output.kind,
            )
        }
        Stop(StopCommand::Host(cmd)) => {
            let output = cmd.output;
            sp =
                update_spinner_message(sp, format!(" Stopping host {} ... ", cmd.host_id), &output);
            let ack = stop_host(cmd.clone()).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Host {} acknowledged stop request", cmd.host_id),
                &ack.error,
                &output.kind,
            )
        }
        Update(UpdateCommand::Actor(cmd)) => {
            let output = cmd.output;
            sp = update_spinner_message(
                sp,
                format!(
                    " Updating Actor {} to {} ... ",
                    cmd.actor_id, cmd.new_actor_ref
                ),
                &output,
            );
            let ack = update_actor(cmd.clone()).await?;
            ctl_operation_output(
                ack.accepted,
                &format!("Actor {} updated to {}", cmd.actor_id, cmd.new_actor_ref),
                &ack.error,
                &output.kind,
            )
        }
    };

    if sp.is_some() {
        sp.unwrap().stop()
    }

    Ok(out)
}

pub(crate) async fn get_hosts(cmd: GetHostsCommand) -> Result<Vec<Host>> {
    let timeout = Duration::from_millis(cmd.opts.timeout_ms.unwrap_or(DEFAULT_NATS_TIMEOUT));
    let client = ctl_client_from_opts(cmd.opts).await?;
    client.get_hosts(timeout).await.map_err(convert_error)
}

pub(crate) async fn get_host_inventory(cmd: GetHostInventoryCommand) -> Result<HostInventory> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .get_host_inventory(&cmd.host_id.to_string())
        .await
        .map_err(convert_error)
}

pub(crate) async fn get_claims(cmd: GetClaimsCommand) -> Result<GetClaimsResponse> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client.get_claims().await.map_err(convert_error)
}

pub(crate) async fn link_del(cmd: LinkDelCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .remove_link(
            &cmd.actor_id.to_string(),
            &cmd.contract_id,
            &cmd.link_name.unwrap_or_else(|| "default".to_string()),
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn link_put(cmd: LinkPutCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .advertise_link(
            &cmd.actor_id.to_string(),
            &cmd.provider_id.to_string(),
            &cmd.contract_id,
            &cmd.link_name.unwrap_or_else(|| "default".to_string()),
            labels_vec_to_hashmap(cmd.values)?,
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn link_query(cmd: LinkQueryCommand) -> Result<LinkDefinitionList> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client.query_links().await.map_err(convert_error)
}

pub(crate) async fn start_actor(cmd: StartActorCommand) -> Result<CtlOperationAck> {
    // If timeout isn't supplied, override with a reasonably long timeout to account for
    // OCI downloads and response
    let opts = if cmd.opts.timeout_ms.is_none() {
        ConnectionOpts {
            timeout_ms: Some(15000),
            ..cmd.opts
        }
    } else {
        cmd.opts
    };
    let client = ctl_client_from_opts(opts).await?;

    let host = match cmd.host_id {
        Some(host) => host,
        None => {
            let suitable_hosts = client
                .perform_actor_auction(
                    &cmd.actor_ref,
                    labels_vec_to_hashmap(cmd.constraints.unwrap_or_default())?,
                    Duration::from_millis(cmd.auction_timeout_ms.unwrap_or(DEFAULT_NATS_TIMEOUT)),
                )
                .await
                .map_err(convert_error)?;
            if suitable_hosts.is_empty() {
                return Err(format!("No suitable hosts found for actor {}", cmd.actor_ref).into());
            } else {
                suitable_hosts[0].host_id.parse()?
            }
        }
    };

    client
        .start_actor(&host.to_string(), &cmd.actor_ref, None)
        .await
        .map_err(convert_error)
}

pub(crate) async fn start_provider(cmd: StartProviderCommand) -> Result<CtlOperationAck> {
    // If timeout isn't supplied, override with a reasonably long timeout to account for
    // OCI downloads and response
    let opts = if cmd.opts.timeout_ms.is_none() {
        ConnectionOpts {
            timeout_ms: Some(60000),
            ..cmd.opts
        }
    } else {
        cmd.opts
    };
    let client = ctl_client_from_opts(opts).await?;

    let host = match cmd.host_id {
        Some(host) => host,
        None => {
            let suitable_hosts = client
                .perform_provider_auction(
                    &cmd.provider_ref,
                    &cmd.link_name,
                    labels_vec_to_hashmap(cmd.constraints.unwrap_or_default())?,
                    Duration::from_millis(cmd.auction_timeout_ms.unwrap_or(DEFAULT_NATS_TIMEOUT)),
                )
                .await
                .map_err(convert_error)?;
            if suitable_hosts.is_empty() {
                return Err(
                    format!("No suitable hosts found for provider {}", cmd.provider_ref).into(),
                );
            } else {
                suitable_hosts[0].host_id.parse()?
            }
        }
    };

    client
        .start_provider(
            &host.to_string(),
            &cmd.provider_ref,
            Some(cmd.link_name),
            None,
            None,
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn stop_provider(cmd: StopProviderCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .stop_provider(
            &cmd.host_id.to_string(),
            &cmd.provider_id.to_string(),
            &cmd.link_name,
            &cmd.contract_id,
            None,
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn stop_actor(cmd: StopActorCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .stop_actor(
            &cmd.host_id.to_string(),
            &cmd.actor_id.to_string(),
            cmd.count,
            None,
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn stop_host(cmd: StopHostCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .stop_host(&cmd.host_id.to_string(), cmd.host_shutdown_timeout)
        .await
        .map_err(convert_error)
}

pub(crate) async fn update_actor(cmd: UpdateActorCommand) -> Result<CtlOperationAck> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    client
        .update_actor(
            &cmd.host_id.to_string(),
            &cmd.actor_id.to_string(),
            &cmd.new_actor_ref,
            None,
        )
        .await
        .map_err(convert_error)
}

pub(crate) async fn apply_manifest(cmd: ApplyCommand) -> Result<Vec<String>> {
    let client = ctl_client_from_opts(cmd.opts).await?;
    let hm = match HostManifest::from_path(Path::new(&cmd.path), cmd.expand_env) {
        Ok(hm) => hm,
        Err(e) => return Err(format!("Failed to load manifest: {}", e).into()),
    };
    let mut results = vec![];
    results.extend_from_slice(&apply_manifest_actors(&cmd.host_key, &client, &hm).await?);
    results.extend_from_slice(&apply_manifest_providers(&cmd.host_key, &client, &hm).await?);
    results.extend_from_slice(&apply_manifest_linkdefs(&client, &hm).await?);
    Ok(results)
}

async fn apply_manifest_actors(
    host_id: &ServerId,
    client: &CtlClient,
    hm: &HostManifest,
) -> Result<Vec<String>> {
    let mut results = vec![];

    for actor in hm.actors.iter() {
        match client.start_actor(&host_id.to_string(), actor, None).await {
            Ok(ack) => {
                if ack.accepted {
                    results.push(format!(
                        "Instruction to start actor {} acknowledged.",
                        actor
                    ));
                } else {
                    results.push(format!(
                        "Instruction to start actor {} not acked: {}",
                        actor, ack.error
                    ));
                }
            }
            Err(e) => results.push(format!("Failed to send start actor: {}", e)),
        }
    }

    Ok(results)
}

async fn apply_manifest_linkdefs(client: &CtlClient, hm: &HostManifest) -> Result<Vec<String>> {
    let mut results = vec![];

    for ld in hm.links.iter() {
        match client
            .advertise_link(
                &ld.actor,
                &ld.provider_id,
                &ld.contract_id,
                ld.link_name.as_ref().unwrap_or(&"default".to_string()),
                ld.values.clone().unwrap_or_default(),
            )
            .await
        {
            Ok(ack) => {
                if ack.accepted {
                    results.push(format!(
                        "Link def submission from {} to {} acknowledged.",
                        ld.actor, ld.provider_id
                    ));
                } else {
                    results.push(format!(
                        "Link def submission from {} to {} not acked: {}",
                        ld.actor, ld.provider_id, ack.error
                    ));
                }
            }
            Err(e) => results.push(format!("Failed to send link def: {}", e)),
        }
    }

    Ok(results)
}

async fn apply_manifest_providers(
    host_id: &ServerId,
    client: &CtlClient,
    hm: &HostManifest,
) -> Result<Vec<String>> {
    let mut results = vec![];

    for cap in hm.capabilities.iter() {
        match client
            .start_provider(
                &host_id.to_string(),
                &cap.image_ref,
                cap.link_name.clone(),
                None,
                None,
            )
            .await
        {
            Ok(ack) => {
                if ack.accepted {
                    results.push(format!(
                        "Instruction to start provider {} acknowledged.",
                        cap.image_ref
                    ));
                } else {
                    results.push(format!(
                        "Instruction to start provider {} not acked: {}",
                        cap.image_ref, ack.error
                    ));
                }
            }
            Err(e) => results.push(format!("Failed to send start capability message: {}", e)),
        }
    }

    Ok(results)
}

async fn ctl_client_from_opts(opts: ConnectionOpts) -> Result<CtlClient> {
    // Attempt to load a context, falling back on the default if not supplied
    let ctx = if let Some(context) = opts.context {
        load_context(&context).ok()
    } else if let Ok(ctx_dir) = context_dir(None) {
        get_default_context(&ctx_dir).ok()
    } else {
        None
    };

    // Determine connection parameters, taking explicitly provided flags,
    // then provided context values, lastly using defaults

    let timeout = opts.timeout_ms.unwrap_or_else(|| {
        ctx.as_ref()
            .map(|c| c.ctl_timeout)
            .unwrap_or(DEFAULT_NATS_TIMEOUT)
    });

    let lattice_prefix = opts.lattice_prefix.unwrap_or_else(|| {
        ctx.as_ref()
            .map(|c| c.ctl_lattice_prefix.clone())
            .unwrap_or_else(|| DEFAULT_LATTICE_PREFIX.to_string())
    });

    let ctl_host = opts.ctl_host.unwrap_or_else(|| {
        ctx.as_ref()
            .map(|c| c.ctl_host.clone())
            .unwrap_or_else(|| DEFAULT_NATS_HOST.to_string())
    });

    let ctl_port = opts.ctl_port.unwrap_or_else(|| {
        ctx.as_ref()
            .map(|c| c.ctl_port.to_string())
            .unwrap_or_else(|| DEFAULT_NATS_PORT.to_string())
    });

    let ctl_jwt = if opts.ctl_jwt.is_some() {
        opts.ctl_jwt
    } else {
        ctx.as_ref().map(|c| c.ctl_jwt.clone()).unwrap_or_default()
    };

    let ctl_seed = if opts.ctl_seed.is_some() {
        opts.ctl_seed
    } else {
        ctx.as_ref().map(|c| c.ctl_seed.clone()).unwrap_or_default()
    };

    let ctl_credsfile = if opts.ctl_credsfile.is_some() {
        opts.ctl_credsfile
    } else {
        ctx.as_ref()
            .map(|c| c.ctl_credsfile.clone())
            .unwrap_or_default()
    };

    let nc =
        crate::util::nats_client_from_opts(&ctl_host, &ctl_port, ctl_jwt, ctl_seed, ctl_credsfile)
            .await?;
    let ctl_client = CtlClient::new(nc, Some(lattice_prefix), Duration::from_secs(timeout));

    Ok(ctl_client)
}

/// Handles updating the spinner for text output
/// JSON output will be corrupted with a spinner
fn update_spinner_message(
    spinner: Option<Spinner>,
    msg: String,
    output: &Output,
) -> Option<Spinner> {
    if let Some(sp) = spinner {
        sp.message(msg);
        Some(sp)
    } else if matches!(output.kind, OutputKind::Text) {
        Some(Spinner::new(&Spinners::Dots12, msg))
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const CTL_HOST: &str = "127.0.0.1";
    const CTL_PORT: &str = "4222";
    const LATTICE_PREFIX: &str = "default";

    const ACTOR_ID: &str = "MDPDJEYIAK6MACO67PRFGOSSLODBISK4SCEYDY3HEOY4P5CVJN6UCWUK";
    const PROVIDER_ID: &str = "VBKTSBG2WKP6RJWLQ5O7RDVIIB4LMW6U5R67A7QMIDBZDGZWYTUE3TSI";
    const HOST_ID: &str = "NCE7YHGI42RWEKBRDJZWXBEJJCFNE5YIWYMSTLGHQBEGFY55BKJ3EG3G";

    #[test]
    /// Enumerates multiple options of the `ctl` command to ensure API doesn't
    /// change between versions. This test will fail if any subcommand of `wash ctl`
    /// changes syntax, ordering of required elements, or flags.
    fn test_ctl_comprehensive() -> Result<()> {
        let start_actor_all = CtlCli::from_iter_safe(&[
            "ctl",
            "start",
            "actor",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            "--auction-timeout-ms",
            "2000",
            "--constraint",
            "arch=x86_64",
            "--host-id",
            HOST_ID,
            "wasmcloud.azurecr.io/actor:v1",
        ])?;
        match start_actor_all.command {
            CtlCliCommand::Start(StartCommand::Actor(super::StartActorCommand {
                opts,
                output,
                host_id,
                actor_ref,
                constraints,
                auction_timeout_ms,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(auction_timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(host_id.unwrap(), HOST_ID.parse()?);
                assert_eq!(actor_ref, "wasmcloud.azurecr.io/actor:v1".to_string());
                assert_eq!(constraints.unwrap(), vec!["arch=x86_64".to_string()]);
            }
            cmd => panic!("ctl start actor constructed incorrect command {:?}", cmd),
        }
        let start_provider_all = CtlCli::from_iter_safe(&[
            "ctl",
            "start",
            "provider",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            "--auction-timeout-ms",
            "2000",
            "--constraint",
            "arch=x86_64",
            "--host-id",
            HOST_ID,
            "--link-name",
            "default",
            "wasmcloud.azurecr.io/provider:v1",
        ])?;
        match start_provider_all.command {
            CtlCliCommand::Start(StartCommand::Provider(super::StartProviderCommand {
                opts,
                output,
                host_id,
                provider_ref,
                link_name,
                constraints,
                auction_timeout_ms,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(auction_timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(link_name, "default".to_string());
                assert_eq!(constraints.unwrap(), vec!["arch=x86_64".to_string()]);
                assert_eq!(host_id.unwrap(), HOST_ID.parse()?);
                assert_eq!(provider_ref, "wasmcloud.azurecr.io/provider:v1".to_string());
            }
            cmd => panic!("ctl start provider constructed incorrect command {:?}", cmd),
        }
        let stop_actor_all = CtlCli::from_iter_safe(&[
            "ctl",
            "stop",
            "actor",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            "--count",
            "2",
            HOST_ID,
            ACTOR_ID,
        ])?;
        match stop_actor_all.command {
            CtlCliCommand::Stop(StopCommand::Actor(super::StopActorCommand {
                opts,
                output,
                host_id,
                actor_id,
                count,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(host_id, HOST_ID.parse()?);
                assert_eq!(actor_id, ACTOR_ID.parse()?);
                assert_eq!(count, 2);
            }
            cmd => panic!("ctl stop actor constructed incorrect command {:?}", cmd),
        }
        let stop_provider_all = CtlCli::from_iter_safe(&[
            "ctl",
            "stop",
            "provider",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            HOST_ID,
            PROVIDER_ID,
            "default",
            "wasmcloud:provider",
        ])?;
        match stop_provider_all.command {
            CtlCliCommand::Stop(StopCommand::Provider(super::StopProviderCommand {
                opts,
                output,
                host_id,
                provider_id,
                link_name,
                contract_id,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(host_id, HOST_ID.parse()?);
                assert_eq!(provider_id, PROVIDER_ID.parse()?);
                assert_eq!(link_name, "default".to_string());
                assert_eq!(contract_id, "wasmcloud:provider".to_string());
            }
            cmd => panic!("ctl stop actor constructed incorrect command {:?}", cmd),
        }
        let get_hosts_all = CtlCli::from_iter_safe(&[
            "ctl",
            "get",
            "hosts",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
        ])?;
        match get_hosts_all.command {
            CtlCliCommand::Get(GetCommand::Hosts(GetHostsCommand { opts, output })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
            }
            cmd => panic!("ctl get hosts constructed incorrect command {:?}", cmd),
        }
        let get_host_inventory_all = CtlCli::from_iter_safe(&[
            "ctl",
            "get",
            "inventory",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            HOST_ID,
        ])?;
        match get_host_inventory_all.command {
            CtlCliCommand::Get(GetCommand::HostInventory(GetHostInventoryCommand {
                opts,
                output,
                host_id,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(host_id, HOST_ID.parse()?);
            }
            cmd => panic!("ctl get inventory constructed incorrect command {:?}", cmd),
        }
        let get_claims_all = CtlCli::from_iter_safe(&[
            "ctl",
            "get",
            "claims",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
        ])?;
        match get_claims_all.command {
            CtlCliCommand::Get(GetCommand::Claims(GetClaimsCommand { opts, output })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
            }
            cmd => panic!("ctl get claims constructed incorrect command {:?}", cmd),
        }
        let link_all = CtlCli::from_iter_safe(&[
            "ctl",
            "link",
            "put",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            "--link-name",
            "default",
            ACTOR_ID,
            PROVIDER_ID,
            "wasmcloud:provider",
            "THING=foo",
        ])?;
        match link_all.command {
            CtlCliCommand::Link(LinkCommand::Put(LinkPutCommand {
                opts,
                output,
                actor_id,
                provider_id,
                contract_id,
                link_name,
                values,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(actor_id, ACTOR_ID.parse()?);
                assert_eq!(provider_id, PROVIDER_ID.parse()?);
                assert_eq!(contract_id, "wasmcloud:provider".to_string());
                assert_eq!(link_name.unwrap(), "default".to_string());
                assert_eq!(values, vec!["THING=foo".to_string()]);
            }
            cmd => panic!("ctl link put constructed incorrect command {:?}", cmd),
        }
        let update_all = CtlCli::from_iter_safe(&[
            "ctl",
            "update",
            "actor",
            "-o",
            "json",
            "--lattice-prefix",
            LATTICE_PREFIX,
            "--ctl-host",
            CTL_HOST,
            "--ctl-port",
            CTL_PORT,
            "--timeout-ms",
            "2000",
            HOST_ID,
            ACTOR_ID,
            "wasmcloud.azurecr.io/actor:v2",
        ])?;
        match update_all.command {
            CtlCliCommand::Update(UpdateCommand::Actor(super::UpdateActorCommand {
                opts,
                output,
                host_id,
                actor_id,
                new_actor_ref,
            })) => {
                assert_eq!(&opts.ctl_host.unwrap(), CTL_HOST);
                assert_eq!(&opts.ctl_port.unwrap(), CTL_PORT);
                assert_eq!(&opts.lattice_prefix.unwrap(), LATTICE_PREFIX);
                assert_eq!(opts.timeout_ms.unwrap(), 2000);
                assert_eq!(output.kind, OutputKind::Json);
                assert_eq!(host_id, HOST_ID.parse()?);
                assert_eq!(actor_id, ACTOR_ID.parse()?);
                assert_eq!(new_actor_ref, "wasmcloud.azurecr.io/actor:v2".to_string());
            }
            cmd => panic!("ctl get claims constructed incorrect command {:?}", cmd),
        }

        Ok(())
    }
}
