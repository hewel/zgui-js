import { createContext, omit, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { sheet } from "./generated/TableStyle.js";
import { element, part, provide, slot, useStylesheet, withDefaults } from "./shared.js";

export interface TableProps extends NativeElementProps {
  readonly columns?: string | undefined;
  readonly stickyHeader?: boolean | undefined;
}

export interface TableCellProps extends NativeElementProps {
  readonly align?: "start" | "center" | "end" | undefined;
}

interface TableScope {
  readonly sticky?: (() => boolean | undefined) | undefined;
  readonly selected?: () => boolean | null | undefined;
}

export const TableContext = createContext<TableScope>({});

export function Table(props: TableProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "columns", "stickyHeader", "children");
  const node = element(
    "scroll",
    mergeProps(rest, withDefaults({ role: "table" as const }, rest), {
      __zguiClass: `${sheet.className} zui-table-container`,
      get __zguiStyle() {
        return { "--zui-table-columns": props.columns };
      },
    }),
  );
  slot(node, provide(TableContext, { sticky: () => props.stickyHeader }, props));
  return node;
}

export const TableHeader = part("box", sheet, "zui-table__section", { role: "rowGroup" });
export const TableBody = part("box", sheet, "zui-table__section zui-table__body", {
  role: "rowGroup",
});
export const TableFooter = part("box", sheet, "zui-table__section zui-table__footer", {
  role: "rowGroup",
});
export const TableCaption = part("box", sheet, "zui-table__caption", { role: "caption" });

const row = part("box", sheet, "zui-table__row", { role: "row" });
const head = part("box", sheet, "zui-table__head", { role: "columnHeader" });
const cell = part("box", sheet, "zui-table__cell", { role: "cell" });

export function TableRow(props: NativeElementProps = {}): NativeNode {
  const { sticky } = useContext(TableContext);
  const node = row(omit(props, "children"));
  slot(node, provide(TableContext, { sticky, selected: () => props.selected }, props));
  return node;
}

function aligned(props: TableCellProps): NativeElementProps {
  return mergeProps(omit(props, "align"), {
    get "data-align"() {
      return props.align;
    },
  });
}

export function TableHead(props: TableCellProps = {}): NativeNode {
  const { sticky } = useContext(TableContext);
  return head(
    mergeProps(aligned(props), {
      get "data-sticky"() {
        return sticky?.() ? "true" : undefined;
      },
    }),
  );
}

export function TableCell(props: TableCellProps = {}): NativeNode {
  const { selected } = useContext(TableContext);
  return cell(
    mergeProps(aligned(props), {
      get "data-selected"() {
        return selected?.() ? "true" : undefined;
      },
    }),
  );
}
