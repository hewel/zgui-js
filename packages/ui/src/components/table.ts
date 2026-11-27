import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { sheet } from "./generated/TableStyle.js";
import { element, part, useStylesheet, withDefaults } from "./shared.js";

export interface TableProps extends NativeElementProps {
  readonly columns?: string | undefined;
  readonly stickyHeader?: boolean | undefined;
}

export interface TableCellProps extends NativeElementProps {
  readonly align?: "start" | "center" | "end" | undefined;
}

export function Table(props: TableProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "columns", "stickyHeader");
  return element(
    "scroll",
    mergeProps(rest, withDefaults({ role: "table" as const }, rest), {
      __zguiClass: `${sheet.className} zui-table-container`,
      get __zguiStyle() {
        return { "--zui-table-columns": props.columns };
      },
      get "data-sticky-header"() {
        return props.stickyHeader ? "true" : undefined;
      },
    }),
  );
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
  return row(
    mergeProps(props, {
      get "data-selected"() {
        return props.selected ? "true" : undefined;
      },
    }),
  );
}

function aligned(props: TableCellProps): NativeElementProps {
  return mergeProps(omit(props, "align"), {
    get "data-align"() {
      return props.align;
    },
  });
}

export function TableHead(props: TableCellProps = {}): NativeNode {
  return head(aligned(props));
}

export function TableCell(props: TableCellProps = {}): NativeNode {
  return cell(aligned(props));
}
