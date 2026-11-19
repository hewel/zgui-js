declare module "@babel/preset-typescript" {
  import type { ConfigAPI, TransformOptions } from "@babel/core";

  export interface Options {
    onlyRemoveTypeImports?: boolean;
    allExtensions?: boolean;
    isTSX?: boolean;
  }

  export default function typescriptPreset(api: ConfigAPI, options?: Options): TransformOptions;
}

declare module "@solidjs/babel-plugin" {
  import type { ConfigAPI, PluginObj } from "@babel/core";

  export interface Options {
    generate?: "universal";
    moduleName?: string;
    dev?: boolean;
  }

  export default function solidBabelPlugin(api: ConfigAPI, options?: Options): PluginObj;
}
