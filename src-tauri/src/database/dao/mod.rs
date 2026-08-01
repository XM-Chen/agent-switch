//! Data Access Object layer
//!
//! Database access operations for each domain

pub mod custom_aggregates;
pub mod failover;
pub mod gateway_auth;
pub mod gateway_control;
pub mod gateway_domain;
pub mod provider_models;
pub mod providers;
pub mod providers_seed;
pub mod proxy;
pub mod settings;
pub mod universal_providers;
pub mod usage_rollup;

// 所有 DAO 方法都通过 Database impl 提供，无需单独导出
pub use gateway_auth::GatewayApiKeyRecord;
pub use gateway_control::{
    CreateGatewayModelInput, CreateGatewayUpstreamInput, CreateRouteTargetInput,
    GatewayUpstreamDto, UpdateGatewayModelInput, UpdateGatewayUpstreamInput,
    UpdateRouteTargetInput, UpstreamCredentialHintDto,
};
pub use gateway_domain::{
    GatewayConfigRecord, GatewayMigrationIssue, GatewayModelRecord, ModelAliasRecord,
    RouteTargetHealthRecord, RouteTargetRecord, UpstreamModelRecord, UpstreamRecord,
};

// 导出自定义聚合类型供 service/command 层使用
pub use custom_aggregates::{AggregateRef, CcAggregateConfig, CustomAggregate, TierSelection};
