import assert from "node:assert/strict";

function resolveRef(schema, root) {
  if (!schema.$ref) return schema;
  const target = schema.$ref.slice("#/$defs/".length);
  const resolved = root.$defs?.[target];
  assert.ok(resolved, `missing schema $def ${target}`);
  return resolved;
}

/** Validate the subset of JSON Schema used by the package fixture. */
export function validateAgainstSchema(value, rawSchema, root, ptr = "$") {
  const schema = resolveRef(rawSchema, root);
  if (schema.const !== undefined) {
    assert.equal(value, schema.const, `${ptr}: expected const ${JSON.stringify(schema.const)}`);
    return;
  }
  if (schema.enum !== undefined) {
    assert.ok(schema.enum.includes(value), `${ptr}: ${JSON.stringify(value)} not in enum`);
    return;
  }
  if (schema.anyOf) {
    const failures = [];
    for (const branch of schema.anyOf) {
      try {
        validateAgainstSchema(value, branch, root, ptr);
        return;
      } catch (error) {
        failures.push(error instanceof Error ? error.message : String(error));
      }
    }
    assert.fail(`${ptr}: did not match anyOf (${failures.join("; ")})`);
  }
  if (schema.type === "boolean") {
    assert.equal(typeof value, "boolean", `${ptr}: expected boolean`);
    return;
  }
  if (schema.type === "integer") {
    assert.ok(Number.isInteger(value), `${ptr}: expected integer`);
    if (schema.minimum !== undefined) assert.ok(value >= schema.minimum, `${ptr}: below minimum`);
    if (schema.maximum !== undefined) assert.ok(value <= schema.maximum, `${ptr}: above maximum`);
    return;
  }
  if (schema.type === "string" || typeof value === "string") {
    assert.equal(typeof value, "string", `${ptr}: expected string`);
    if (schema.minLength !== undefined)
      assert.ok(value.length >= schema.minLength, `${ptr}: too short`);
    if (schema.maxLength !== undefined)
      assert.ok(value.length <= schema.maxLength, `${ptr}: too long`);
    if (schema.pattern !== undefined) {
      assert.match(value, new RegExp(schema.pattern), `${ptr}: failed pattern ${schema.pattern}`);
    }
    if (!schema.anyOf) return;
  }
  if (schema.type === "array") {
    assert.ok(Array.isArray(value), `${ptr}: expected array`);
    if (schema.maxItems !== undefined)
      assert.ok(value.length <= schema.maxItems, `${ptr}: maxItems`);
    if (schema.uniqueItems) {
      const seen = new Set(value.map(v => JSON.stringify(v)));
      assert.equal(seen.size, value.length, `${ptr}: expected uniqueItems`);
    }
    if (schema.items) {
      value.forEach((item, idx) =>
        validateAgainstSchema(item, schema.items, root, `${ptr}[${idx}]`),
      );
    }
    return;
  }
  if (schema.type === "object") {
    assert.ok(
      value !== null && typeof value === "object" && !Array.isArray(value),
      `${ptr}: expected object`,
    );
    for (const req of schema.required ?? []) {
      assert.ok(req in value, `${ptr}: missing required property ${req}`);
    }
    for (const [key, val] of Object.entries(value)) {
      if (schema.propertyNames) {
        validateAgainstSchema(key, schema.propertyNames, root, `${ptr}.${key}(name)`);
      }
      if (schema.properties && key in schema.properties) {
        validateAgainstSchema(val, schema.properties[key], root, `${ptr}.${key}`);
      } else if (schema.additionalProperties === false) {
        assert.fail(`${ptr}: unexpected property ${key}`);
      } else if (schema.additionalProperties && typeof schema.additionalProperties === "object") {
        validateAgainstSchema(val, schema.additionalProperties, root, `${ptr}.${key}`);
      }
    }
  }
}
