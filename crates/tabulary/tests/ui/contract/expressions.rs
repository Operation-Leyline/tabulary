use tabulary::{Attr, Converter, Mutable, Record, Required, Update, wire};

fn owner<R: Record, S: Record>(left: Attr<R, u64>, right: Attr<S, u64>) {
    left.eq_path(right);
    left.eq(1).and(right.eq(1));
    Update::<R>::new().and(right.set(1));
    left.set_expr(right.operand());
}

fn logical_type<R: Record>(field: Attr<R, u64>) {
    field.eq("3".to_owned());
    field.set("3".to_owned());
}

fn codec<R: Record, C: Converter<u64, Wire = wire::Number> + 'static>(
    left: Attr<R, u64>,
    right: Attr<R, u64, Required, Mutable, C>,
) {
    left.eq_path(right);
    left.set_expr(right.if_not_exists(0));
}

fn main() {}
