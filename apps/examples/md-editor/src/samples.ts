export interface MarkdownDoc {
  readonly id: string;
  readonly title: string;
  readonly badge: string;
  readonly content: string;
  readonly source?: "builtin" | "file" | "workspace" | undefined;
  readonly relativePath?: string | undefined;
  readonly dirty?: boolean | undefined;
}

export const INITIAL_DOCS: readonly MarkdownDoc[] = [
  {
    id: "guide",
    title: "zgui-js 架构速览",
    badge: "Guide",
    source: "builtin",
    content: `# zgui-js Markdown Studio

这是一个完全运行在 **Rust + wgpu + Stylo + Parley + QuickJS-NG + Solid 2.0.1** 上的原生本地 Markdown 工作室，**零浏览器内核、零 DOM** 🚀。

> [!TIP]
> 在右侧预览里直接点击任务清单的复选框，会**实时反向修改左侧 Markdown 源码**；也可以用顶部工具栏在当前光标/选区处插入格式语法（原生 UTF-16 选区索引，完美支持中文与 Emoji ✨）。

## 核心渲染管线

![zgui-js 原生渲染管线](diagram://pipeline)

## 能力展示清单

- [x] 分栏实时预览（支持 **Split / Editor / Preview** 三种模式）
- [x] 行内富文本流：**粗体**、*斜体*、~~删除线~~、\`inline code\` 与 [可点击链接](https://github.com/hewel/zgui-js)
- [x] GFM 表格、语法高亮代码块与 GitHub 风格 Callout 提示框
- [ ] 在编辑器里尝试选中一段中文，再点顶部 \`B\` 或 \`Code\` 按钮

## 代码块语法高亮

\`\`\`tsx
import { createSignal, createSpring, VirtualList } from "@hewel/zgui";

export function LiveCounter() {
  const [count, setCount] = createSignal(0);
  const smooth = createSpring(count, { stiffness: 180, damping: 18 });
  return <text>{Math.round(smooth())}</text>;
}
\`\`\`

## 实测对比表

| 组件模块 | 实现方式 | 特点 |
| :--- | :---: | ---: |
| 布局引擎 | Taffy + Parley | 原生 Flex/Grid + CJK 断行 |
| 样式系统 | Stylo + Lightning CSS | CSS Modules + CSS 变量主题 |
| 响应式层 | Solid 2.0.1 | 细粒度更新 + QuickJS-NG |
| 渲染后端 | wgpu + Vello | GPU 加速 + 抗锯齿矢量 |
`,
  },
  {
    id: "kitchen",
    title: "GFM Typography Probe",
    badge: "Probe",
    content: `# Heading 1 — 排版与行内流探测

## Heading 2 — 行内折行混排

这一段故意混合了 **Bold English**、*斜体强调*、\`createDelay()\` 行内代码胶囊、~~旧方案删除线~~ 以及 [Issue #48](https://github.com/hewel/zgui-js/issues/48)，用于测试 Parley + Stylo 在窄栏和宽栏下的连续折行与基线对齐表现。

### Heading 3 — 五种 GitHub Callout

> [!NOTE]
> 这是一条 Note 提示：用于补充背景上下文与设计说明。

> [!IMPORTANT]
> 这是一条 Important 警告：核心契约与安全红线必须保持 fail-closed。

> [!WARNING]
> 这是一条 Warning 提示：修改 \`vendor/zgui-paint\` 后需同步更新 \`vendor/README.md\`。

> [!CAUTION]
> 这是一条 Caution 提示：切勿在非 owner 作用域泄漏原生句柄。

---

> “简单、清晰的抽象往往比复杂的黑盒优化更经得起考验。”

\`\`\`rust
pub fn render_frame(dt_ms: f32) -> bool {
    // 零拷贝原生帧驱动示例
    let clamped = dt_ms.min(32.0);
    clamped > 0.0
}
\`\`\`
`,
  },
  {
    id: "checklist",
    title: "发布自检清单",
    badge: "Tasks",
    content: `# 发布自检清单（交互式任务）

点击右侧任意复选框，观察左侧源码与侧栏完成度进度条同步变化：

- [x] 通过 \`bun run lint\` 与 \`bun run typecheck\`
- [x] 通过 \`bun run test:build\` 全量单测
- [x] 通过 \`bun run test:vendor\` 与 Rust clippy
- [x] Headless preview 视觉像素核查与 \`teardown.clean === true\`
- [ ] 完成 \`#48\` 源码副本去重与 SourceMap 懒解码
- [ ] 完成 \`#49\` 构建期组件 CSS 静态提升
- [ ] 完成 \`#50\` Release LTO/Strip 与 PSS 内存对比

> [!TIP]
> 也可以切换到 \`Preview\` 模式当作纯粹的原生文档阅读器使用。
`,
  },
];

export function buildStressMarkdown(): string {
  const sections: string[] = [
    "# Large Markdown Stress Document (120+ Blocks)",
    "",
    "> [!NOTE]",
    "> 用于测试在超过 120 个 Markdown 块（标题、混排段落、代码块、表格、任务列表）下，每次按键触发的解析与原生树更新延迟。",
    "",
  ];
  for (let i = 1; i <= 24; i++) {
    sections.push(`## Section ${i} — 性能与排版压测`);
    sections.push(
      `第 **${i}** 节包含 *italic*、\`token_${i}\`、~~deprecated~~ 与 [Ref #${i}](https://zgui.dev/section/${i}) 的混合行内节点。`,
    );
    sections.push(`- [x] Sub-task ${i}.1 已完成`);
    sections.push(`- [ ] Sub-task ${i}.2 待验证`);
    if (i % 4 === 0) {
      sections.push("```ts");
      sections.push(`const section${i} = { index: ${i}, active: true };`);
      sections.push("```");
    }
    sections.push("");
  }
  return sections.join("\n");
}

export function buildPipelineSvg(dark: boolean): string {
  const bg = dark ? "#111722" : "#f8fafc";
  const box = dark ? "#1e293b" : "#ffffff";
  const fg = dark ? "#e2e8f0" : "#0f172a";
  const sub = dark ? "#94a3b8" : "#475569";
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 150">`,
    `<rect width="640" height="150" rx="12" fill="${bg}"/>`,
    // Stage 1: TSX / Solid 2
    `<rect x="16" y="24" width="132" height="102" rx="10" fill="${box}" stroke="#3b82f6" stroke-width="2"/>`,
    `<circle cx="42" cy="52" r="7" fill="#3b82f6"/>`,
    `<circle cx="64" cy="52" r="5" fill="#60a5fa"/>`,
    `<circle cx="53" cy="68" r="5" fill="#93c5fd"/>`,
    `<text x="82" y="54" font-family="sans-serif" font-size="13" font-weight="700" fill="${fg}">TSX</text>`,
    `<text x="82" y="70" font-family="sans-serif" font-size="11" fill="${sub}">Solid 2.0</text>`,
    `<text x="82" y="106" text-anchor="middle" font-family="monospace" font-size="10" fill="#60a5fa">QuickJS-NG</text>`,
    // Arrow 1
    `<path d="M150 75 L176 75" stroke="#3b82f6" stroke-width="2.5" stroke-linecap="round"/>`,
    `<polygon points="176,70 186,75 176,80" fill="#3b82f6"/>`,
    // Stage 2: Native Tree Bridge
    `<rect x="188" y="24" width="132" height="102" rx="10" fill="${box}" stroke="#10b981" stroke-width="2"/>`,
    `<text x="254" y="56" text-anchor="middle" font-family="sans-serif" font-size="13" font-weight="700" fill="${fg}">ui-bridge</text>`,
    `<text x="254" y="76" text-anchor="middle" font-family="sans-serif" font-size="11" fill="${sub}">Native Tree</text>`,
    `<text x="254" y="106" text-anchor="middle" font-family="monospace" font-size="10" fill="#34d399">Zero DOM</text>`,
    // Arrow 2
    `<path d="M322 75 L348 75" stroke="#10b981" stroke-width="2.5" stroke-linecap="round"/>`,
    `<polygon points="348,70 358,75 348,80" fill="#10b981"/>`,
    // Stage 3: Stylo + Taffy + Parley
    `<rect x="360" y="24" width="132" height="102" rx="10" fill="${box}" stroke="#a855f7" stroke-width="2"/>`,
    `<text x="426" y="56" text-anchor="middle" font-family="sans-serif" font-size="13" font-weight="700" fill="${fg}">Stylo + Taffy</text>`,
    `<text x="426" y="76" text-anchor="middle" font-family="sans-serif" font-size="11" fill="${sub}">Parley Text</text>`,
    `<text x="426" y="106" text-anchor="middle" font-family="monospace" font-size="10" fill="#c084fc">CSS Modules</text>`,
    // Arrow 3
    `<path d="M494 75 L520 75" stroke="#a855f7" stroke-width="2.5" stroke-linecap="round"/>`,
    `<polygon points="520,70 530,75 520,80" fill="#a855f7"/>`,
    // Stage 4: wgpu + Vello
    `<rect x="532" y="24" width="92" height="102" rx="10" fill="${box}" stroke="#f59e0b" stroke-width="2"/>`,
    `<text x="578" y="58" text-anchor="middle" font-family="sans-serif" font-size="13" font-weight="700" fill="${fg}">Vello</text>`,
    `<text x="578" y="78" text-anchor="middle" font-family="sans-serif" font-size="11" fill="${sub}">wgpu</text>`,
    `<text x="578" y="106" text-anchor="middle" font-family="monospace" font-size="10" fill="#fbbf24">Wayland</text>`,
    `</svg>`,
  ].join("");
}

export function encodeUtf8Bytes(text: string): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < text.length; i++) {
    const code = text.codePointAt(i)!;
    if (code > 0xffff) i += 1;
    if (code < 0x80) {
      out.push(code);
    } else if (code < 0x800) {
      out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code < 0x10000) {
      out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    } else {
      out.push(
        0xf0 | (code >> 18),
        0x80 | ((code >> 12) & 0x3f),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f),
      );
    }
  }
  return new Uint8Array(out);
}
