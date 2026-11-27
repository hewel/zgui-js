use js_runtime::{JsRuntime, RuntimeLimits};

#[test]
fn url_and_search_params_follow_whatwg_mutation_rules() {
    let mut runtime = JsRuntime::new(RuntimeLimits::default()).unwrap();
    runtime
        .with_context(|ctx| {
            ctx.eval::<(), _>(
                r#"
                const check = (actual, expected) => {
                    if (JSON.stringify(actual) !== JSON.stringify(expected))
                        throw Error(`${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
                };
                const url = new URL('../a?z=1&n=two%20words&z=2#', 'HTTPS://u:p@bücher.example:443/dir/x');
                const params = url.searchParams;
                check(url.href, 'https://u:p@xn--bcher-kva.example/a?z=1&n=two%20words&z=2#');
                check([url.origin, url.protocol, url.username, url.password, url.host, url.hostname,
                       url.port, url.pathname, url.search, url.hash],
                      ['https://xn--bcher-kva.example', 'https:', 'u', 'p', 'xn--bcher-kva.example',
                       'xn--bcher-kva.example', '', '/a', '?z=1&n=two%20words&z=2', '']);
                params.delete('z', '1');
                params.append('n', '✓');
                params.sort();
                check([...params], [['n', 'two words'], ['n', '✓'], ['z', '2']]);
                check(params.getAll('n'), ['two words', '✓']);
                check([params.get('missing'), params.has('n', '✓'), params.size], [null, true, 3]);
                check(url.search, '?n=two+words&n=%E2%9C%93&z=2');
                url.search = '?a=1&a=2';
                check(url.searchParams === params, true);
                params.set('a', '3');
                check([...params.keys()], ['a']);
                check([...params.values()], ['3']);
                const calls = [];
                params.forEach((value, key, list) => calls.push([key, value, list === params]));
                check(calls, [['a', '3', true]]);
                url.host = 'example.org:8443';
                url.pathname = '/a b';
                url.hash = 'a b';
                check(url.toJSON(), 'https://u:p@example.org:8443/a%20b?a=3#a%20b');
                url.href = 'data:text/plain,hello';
                check([url.toString(), url.origin, params.size], ['data:text/plain,hello', 'null', 0]);
                check([URL.canParse('bad'), URL.canParse('bad', 'https://example.com'),
                       URL.canParse('https://example.com', 'bad')], [false, true, false]);
                check(String(new URLSearchParams({ x: 'a + b', y: '\ud800' })), 'x=a+%2B+b&y=%EF%BF%BD');
                check(String(new URLSearchParams([['a', '1'], ['a', '2']])), 'a=1&a=2');
                const live = new URL('https://example.com/?a=1&b=2');
                const cursor = live.searchParams.entries();
                for (const entry of cursor) { check(entry, ['a', '1']); break; }
                live.search = '?c=3&d=4';
                check(cursor.next().value, ['d', '4']);
                check(cursor.next().done, true);
                live.searchParams.append('e', '5');
                check(cursor.next().value, ['e', '5']);
                check(URL.parse('x', live).href, 'https://example.com/x');
                check(URL.parse('x'), null);
                "#,
            )
        })
        .unwrap();
}

#[test]
fn url_search_params_serialization_rejects_an_oversized_array() {
    let mut runtime = JsRuntime::new(RuntimeLimits::default()).unwrap();
    let name = runtime
        .with_context(|ctx| {
            ctx.eval::<String, _>(
                r#"
                const pairs = new URLSearchParams('a=b&'.repeat(65_537));
                try { pairs.toString(); 'unbounded'; }
                catch (error) { error.name; }
                "#,
            )
        })
        .unwrap();
    assert_eq!(name, "RangeError");
}

#[test]
fn utf8_encoding_preserves_scalar_boundaries_and_decoder_stream_state() {
    let mut runtime = JsRuntime::new(RuntimeLimits::default()).unwrap();
    runtime
        .with_context(|ctx| {
            ctx.eval::<(), _>(
                r#"
                const check = (actual, expected) => {
                    if (JSON.stringify(actual) !== JSON.stringify(expected))
                        throw Error(`${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
                };
                const throws = (fn, kind) => {
                    try { fn(); } catch (e) { if (e instanceof kind) return; throw e; }
                    throw Error(`Expected ${kind.name}`);
                };
                const encoder = new TextEncoder();
                check(encoder.encoding, 'utf-8');
                check([...encoder.encode('\ud800A\udc00')], [239, 191, 189, 65, 239, 191, 189]);
                const destination = new Uint8Array(7).fill(9);
                check(encoder.encodeInto('A😀界', destination.subarray(1)), {read: 3, written: 5});
                check([...destination], [9, 65, 240, 159, 152, 128, 9]);
                check(encoder.encodeInto('😀', destination.subarray(0, 3)), {read: 0, written: 0});
                throws(() => encoder.encodeInto('x', new Int8Array(8)), TypeError);
                throws(() => new TextDecoder('latin1'), RangeError);
                const decoder = new TextDecoder(' \tUTF8\n');
                check([decoder.encoding, decoder.fatal, decoder.ignoreBOM], ['utf-8', false, false]);
                const bytes = encoder.encode('\ufeff中文😀');
                check(decoder.decode(bytes.subarray(0, 1), {stream: true}), '');
                check(decoder.decode(bytes.subarray(1, 5), {stream: true}), '');
                check(decoder.decode(new DataView(bytes.buffer, 5, 4), {stream: true}), '中文');
                check(decoder.decode(bytes.subarray(9)), '😀');
                check(decoder.decode(bytes.buffer), '中文😀');
                check(new TextDecoder('utf-8', {ignoreBOM: true}).decode(bytes), '\ufeff中文😀');
                check(decoder.decode(Uint8Array.of(0xed, 0xa0, 0x80, 0xe1, 0x80, 65, 0xf0)), '����A�');
                const fatal = new TextDecoder('utf-8', {fatal: true});
                check(fatal.decode(Uint8Array.of(0xe4), {stream: true}), '');
                throws(() => fatal.decode(), TypeError);
                check(fatal.decode(encoder.encode('ok')), 'ok');
                check(fatal.decode(encoder.encode('prefix'), {stream: true}), 'prefix');
                throws(() => fatal.decode(Uint8Array.of(0xff), {stream: true}), TypeError);
                check(fatal.decode(encoder.encode('\ufeffnew stream')), 'new stream');
                throws(() => decoder.decode({ buffer: bytes.buffer, byteLength: 4 }), TypeError);
                "#,
            )
        })
        .unwrap();
}
