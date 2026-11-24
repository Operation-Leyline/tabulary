#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
struct NotAComponent {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(key_only)]
    device: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"))]
struct OptionalComponent {
    #[tabulary(key_only)]
    tenant: Option<String>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"))]
struct ConvertedComponent {
    #[tabulary(key_only, with = tabulary::convert::u128_string)]
    tenant: u128,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"),
    index(ident = ByTenant, name = "by_tenant", kind = global, partition = tenant, project = all))]
struct IndexedComponent {
    #[tabulary(key_only)]
    tenant: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"),
    index(ident = ByDevice, name = "by_device", kind = global,
        partition(name = "gsi1", template = "{tenant:text}#{device:text}"), project = all))]
struct IndexTemplateComponent {
    #[tabulary(key_only)]
    tenant: String,
    device: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"),
    index(ident = ByDevice, name = "by_device", kind = global, partition = device,
        project = include(tenant)))]
struct IncludedComponent {
    #[tabulary(key_only)]
    tenant: String,
    device: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"))]
struct DuplicateOption {
    #[tabulary(key_only, key_only)]
    tenant: String,
}

#[derive(tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Note {
    #[tabulary(key_only)]
    tenant: String,
}

fn main() {}
