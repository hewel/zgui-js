#!/usr/bin/env bun
import { runInitCli } from "@hewel/zgui-cli";

if (import.meta.main) {
  try {
    const code = await runInitCli(process.argv.slice(2));
    if (code !== 0) process.exitCode = code;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
