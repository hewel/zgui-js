//! Web primitives in every QuickJS context, including compute workers.

use rquickjs::{
    Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, TypedArray, Value,
    class::Trace, function::Opt,
};
use url::{Url, form_urlencoded, quirks};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct NativeUrl {
    #[qjs(skip_trace)]
    url: Url,
}

const HREF: u8 = 0;
const ORIGIN: u8 = 1;
const PROTOCOL: u8 = 2;
const USERNAME: u8 = 3;
const PASSWORD: u8 = 4;
const HOST: u8 = 5;
const HOSTNAME: u8 = 6;
const PORT: u8 = 7;
const PATHNAME: u8 = 8;
const SEARCH: u8 = 9;
const HASH: u8 = 10;
const MAX_PARAM_PAIRS: u32 = 65_536;

pub(super) fn install<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    // Each bootstrap script's completion value is its installer function.
    let initialize: Function<'js> = ctx.eval(include_str!("web_url.js"))?;
    let component = Object::new(ctx.clone())?;
    for (name, id) in [
        ("href", HREF),
        ("origin", ORIGIN),
        ("protocol", PROTOCOL),
        ("username", USERNAME),
        ("password", PASSWORD),
        ("host", HOST),
        ("hostname", HOSTNAME),
        ("port", PORT),
        ("pathname", PATHNAME),
        ("search", SEARCH),
        ("hash", HASH),
    ] {
        component.set(name, id)?;
    }
    initialize.call::<_, ()>((
        Function::new(ctx.clone(), parse_url)?,
        Function::new(ctx.clone(), can_parse_url)?,
        Function::new(ctx.clone(), get_url)?,
        Function::new(ctx.clone(), set_url)?,
        Function::new(ctx.clone(), parse_params)?,
        Function::new(ctx.clone(), serialize_params)?,
        component,
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
        HREF => quirks::href(url),
        ORIGIN => return quirks::origin(url).into_js(&ctx),
        PROTOCOL => quirks::protocol(url),
        USERNAME => quirks::username(url),
        PASSWORD => quirks::password(url),
        HOST => quirks::host(url),
        HOSTNAME => quirks::hostname(url),
        PORT => quirks::port(url),
        PATHNAME => quirks::pathname(url),
        SEARCH => quirks::search(url),
        HASH => quirks::hash(url),
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
        HREF => {
            return quirks::set_href(url, &value)
                .map_err(|_| Exception::throw_type(&ctx, "Invalid URL"));
        }
        PROTOCOL => quirks::set_protocol(url, &value),
        USERNAME => quirks::set_username(url, &value),
        PASSWORD => quirks::set_password(url, &value),
        HOST => quirks::set_host(url, &value),
        HOSTNAME => quirks::set_hostname(url, &value),
        PORT => quirks::set_port(url, &value),
        PATHNAME => {
            quirks::set_pathname(url, &value);
            return Ok(());
        }
        SEARCH => {
            quirks::set_search(url, &value);
            return Ok(());
        }
        HASH => {
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
    let len: u32 = pairs.as_object().get("length")?;
    if len > MAX_PARAM_PAIRS {
        return Err(Exception::throw_range(
            pairs.ctx(),
            "too many URLSearchParams pairs",
        ));
    }
    let mut query = form_urlencoded::Serializer::new(String::new());
    for index in 0..len as usize {
        let pair: Array = pairs.get(index)?;
        let key: String = pair.get(0)?;
        let value: String = pair.get(1)?;
        query.append_pair(&key, &value);
    }
    Ok(query.finish())
}
