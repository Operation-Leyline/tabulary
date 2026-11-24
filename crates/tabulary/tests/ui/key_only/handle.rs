#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}"))]
struct Session {
    #[tabulary(key_only)]
    tenant: String,
    device: String,
}

#[derive(tabulary::Projection)]
#[tabulary(record = Session)]
struct Tenant {
    #[tabulary(attribute = SessionAttrs::TENANT)]
    tenant: String,
}

fn main() {
    // A key-only component has no attribute, so it has no handle to filter on.
    let _ = SessionAttrs::TENANT.eq("north".into());
}
