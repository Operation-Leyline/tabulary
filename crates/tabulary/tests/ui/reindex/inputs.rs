#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByRoute, name = "by_route", kind = global,
    partition(name = "gpk", template = "{route:hex}!{position:dec20}"), project = all))]
struct Parcel {
    #[tabulary(partition_key)] id: String,
    route: String,
    position: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Other { #[tabulary(partition_key)] id: String }
fn main() {
    let _ = ParcelByRouteInputs { route: "r".into() };
    let _ = ParcelByRouteInputs::new("r".into(), "wrong".to_owned());
    let _ = tabulary::Update::<Other>::new().reindex(ParcelIndexes::BY_ROUTE,
        ParcelByRouteInputs::new("r".into(), 1));
    let _ = ParcelAttrs::ROUTE.set("stale".into());
    let _ = ParcelAttrs::BY_ROUTE_PARTITION.remove();
}
