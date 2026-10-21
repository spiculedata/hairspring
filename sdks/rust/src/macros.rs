//! The assertion macros. Each call site declares its record as a function-local `static` placed in the catalog
//! section, so the catalog is whatever the linker collected; nothing is registered at run time.
//!
//! `id` and `msg` must be string literals (or `concat!` of literals): they are checked at compile time, and the
//! offline JSON twin of the catalog line is built from them. Ids are 1 to 128 bytes of `[A-Za-z0-9._:/-]`; a message
//! is at most 512 bytes of printable ASCII without `"` or `\`. Zero to four values (anything that casts to `i64`)
//! are captured at the first true and the first false evaluation.

/// The record and its offline twin; evaluates to `&'static Site`.
#[doc(hidden)]
#[macro_export]
macro_rules! __hs_site {
    ($kind:ident, $kind_name:literal, $must_hit:literal, $id:expr, $msg:expr) => {{
        const _: () = {
            $crate::__private::check_id($id);
            $crate::__private::check_msg($msg);
        };
        // The section names end in the layout number (`l1`): see `section::LAYOUT`.
        const SOURCE: $crate::__private::TwinSource = $crate::__private::TwinSource {
            id: $id,
            kind_name: $kind_name,
            must_hit: $must_hit,
            msg: $msg,
            file: ::core::file!(),
            line: ::core::line!(),
        };
        const TWIN_LEN: usize = $crate::__private::twin_len(SOURCE);
        #[used]
        #[cfg_attr(target_vendor = "apple", link_section = "__DATA,__hs_rs_cstr_l1")]
        #[cfg_attr(not(target_vendor = "apple"), link_section = "hs_rust_catstr_l1")]
        static OFFLINE: [u8; TWIN_LEN] = $crate::__private::twin::<TWIN_LEN>(SOURCE);
        #[used]
        #[cfg_attr(target_vendor = "apple", link_section = "__DATA,__hs_rs_cat_l1")]
        #[cfg_attr(not(target_vendor = "apple"), link_section = "hs_rust_catalog_l1")]
        static SITE: $crate::__private::Site = $crate::__private::Site::new(
            $id,
            $msg,
            ::core::file!(),
            ::core::line!(),
            $crate::__private::Kind::$kind,
        );
        let _ = &OFFLINE;
        &SITE
    }};
}

/// Up to four values, as the `[i64; 4]` the record stores; missing ones are zero.
#[doc(hidden)]
#[macro_export]
macro_rules! __hs_vals {
    () => {
        [0i64; 4]
    };
    ($a:expr) => {
        [($a) as i64, 0, 0, 0]
    };
    ($a:expr, $b:expr) => {
        [($a) as i64, ($b) as i64, 0, 0]
    };
    ($a:expr, $b:expr, $c:expr) => {
        [($a) as i64, ($b) as i64, ($c) as i64, 0]
    };
    ($a:expr, $b:expr, $c:expr, $d:expr) => {
        [($a) as i64, ($b) as i64, ($c) as i64, ($d) as i64]
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hs_condition {
    ($kind:ident, $kind_name:literal, $must_hit:literal, $cond:expr, $id:expr, $msg:expr $(, $v:expr)*) => {{
        let site = $crate::__hs_site!($kind, $kind_name, $must_hit, $id, $msg);
        $crate::__private::eval(site, $cond, $crate::__hs_vals!($($v),*));
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hs_point {
    ($kind:ident, $kind_name:literal, $must_hit:literal, $id:expr, $msg:expr $(, $v:expr)*) => {{
        let site = $crate::__hs_site!($kind, $kind_name, $must_hit, $id, $msg);
        $crate::__private::hit(site, $crate::__hs_vals!($($v),*));
    }};
}

/// `always!(cond, "id", "message", values...)`: must hold at every evaluation. A false evaluation fails the trial;
/// never evaluating it is a coverage gap at the run level.
#[macro_export]
macro_rules! always {
    ($cond:expr, $id:expr, $msg:expr $(, $v:expr)* $(,)?) => {
        $crate::__hs_condition!(Always, "always", "true", $cond, $id, $msg $(, $v)*)
    };
}

/// Like `always!`, but never reaching it is fine.
#[macro_export]
macro_rules! always_or_unreachable {
    ($cond:expr, $id:expr, $msg:expr $(, $v:expr)* $(,)?) => {
        $crate::__hs_condition!(AlwaysOrUnreachable, "always_or_unreachable", "false", $cond, $id, $msg $(, $v)*)
    };
}

/// `sometimes!(cond, "id", "message", values...)`: must hold at least once per run. Never fails a trial.
#[macro_export]
macro_rules! sometimes {
    ($cond:expr, $id:expr, $msg:expr $(, $v:expr)* $(,)?) => {
        $crate::__hs_condition!(Sometimes, "sometimes", "true", $cond, $id, $msg $(, $v)*)
    };
}

/// `reachable!("id", "message", values...)`: must be reached at least once per run.
#[macro_export]
macro_rules! reachable {
    ($id:expr, $msg:expr $(, $v:expr)* $(,)?) => {
        $crate::__hs_point!(Reachable, "reachable", "true", $id, $msg $(, $v)*)
    };
}

/// `unreachable_point!("id", "message", values...)`: must never be reached; reaching it fails the trial. (Named so
/// that it does not shadow `std::unreachable!`, which panics.)
#[macro_export]
macro_rules! unreachable_point {
    ($id:expr, $msg:expr $(, $v:expr)* $(,)?) => {
        $crate::__hs_point!(Unreachable, "unreachable", "false", $id, $msg $(, $v)*)
    };
}
