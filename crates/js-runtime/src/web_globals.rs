//! Web primitives in every QuickJS context, including compute workers.

use rquickjs::{
    Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, TypedArray, Value, class::Trace,
    function::Opt,
};
use url::{Url, form_urlencoded, quirks};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct NativeUrl {
    #[qjs(skip_trace)]
    url: Url,
}

pub(super) fn install<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    // Each bootstrap script's completion value is its installer function.
    let initialize: Function<'js> = ctx.eval(include_str!("web_url.js"))?;
    initialize.call::<_, ()>((
        Function::new(ctx.clone(), parse_url)?,
        Function::new(ctx.clone(), can_parse_url)?,
        Function::new(ctx.clone(), get_url)?,
        Function::new(ctx.clone(), set_url)?,
        Function::new(ctx.clone(), parse_params)?,
        Function::new(ctx.clone(), serialize_params)?,
    ))?;
    let initialize: Function<'js> = ctx.eval(include_str!("web_text.js"))?;
    initialize.call((Function::new(ctx.clone(), encode_utf8)?,))
}

fn encode_utf8(ctx: Ctx<'_>, input: String) -> rquickjs::Result<TypedArray<'_, u8>> {
    TypedArray::new(ctx, input.into_bytes())
}

fn parse(input: &str, base: Option<&str>) -> Option<Url> {
    let base = match base {
        Some(base) => Some(Url::parse(base).ok()?),
        None => None,
    };
    Url::options().base_url(base.as_ref()).parse(input).ok()
}

fn can_parse_url(input: String, base: Opt<String>) -> bool {
    parse(&input, base.0.as_deref()).is_some()
}

fn parse_url<'js>(
    ctx: Ctx<'js>,
    input: String,
    base: Opt<String>,
) -> rquickjs::Result<Option<Class<'js, NativeUrl>>> {
    parse(&input, base.0.as_deref())
        .map(|url| Class::instance(ctx, NativeUrl { url }))
        .transpose()
}

fn get_url<'js>(
    ctx: Ctx<'js>,
    url: Class<'js, NativeUrl>,
    key: u8,
) -> rquickjs::Result<Value<'js>> {
    let url = &url.borrow().url;
    let value = match key {
        0 => quirks::href(url),
        1 => return quirks::origin(url).into_js(&ctx),
        2 => quirks::protocol(url),
        3 => quirks::username(url),
        4 => quirks::password(url),
        5 => quirks::host(url),
        6 => quirks::hostname(url),
        7 => quirks::port(url),
        8 => quirks::pathname(url),
        9 => quirks::search(url),
        10 => quirks::hash(url),
        _ => return Err(Exception::throw_internal(&ctx, "unknown URL component")),
    };
    value.into_js(&ctx)
}

fn set_url<'js>(
    ctx: Ctx<'js>,
    url: Class<'js, NativeUrl>,
    key: u8,
    value: String,
) -> rquickjs::Result<()> {
    let url = &mut url.borrow_mut().url;
    let _ = match key {
        0 => {
            return quirks::set_href(url, &value)
                .map_err(|_| Exception::throw_type(&ctx, "Invalid URL"));
        }
        2 => quirks::set_protocol(url, &value),
        3 => quirks::set_username(url, &value),
        4 => quirks::set_password(url, &value),
        5 => quirks::set_host(url, &value),
        6 => quirks::set_hostname(url, &value),
        7 => quirks::set_port(url, &value),
        8 => {
            quirks::set_pathname(url, &value);
            return Ok(());
        }
        9 => {
            quirks::set_search(url, &value);
            return Ok(());
        }
        10 => {
            quirks::set_hash(url, &value);
            return Ok(());
        }
        _ => {
            return Err(Exception::throw_internal(
                &ctx,
                "unknown writable URL component",
            ));
        }
    };
    Ok(())
}

fn parse_params(ctx: Ctx<'_>, query: String) -> rquickjs::Result<Array<'_>> {
    let pairs = Array::new(ctx.clone())?;
    for (i, (key, value)) in form_urlencoded::parse(query.as_bytes()).enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, key.as_ref())?;
        pair.set(1, value.as_ref())?;
        pairs.set(i, pair)?;
    }
    Ok(pairs)
}

fn serialize_params(pairs: Array<'_>) -> rquickjs::Result<String> {
    let mut query = form_urlencoded::Serializer::new(String::new());
    for pair in pairs.iter::<Array<'_>>() {
        let pair = pair?;
        let key: String = pair.get(0)?;
        let value: String = pair.get(1)?;
        query.append_pair(&key, &value);
    }
    Ok(query.finish())
}
