import { createSignal } from "@hewel/zgui";
import {
  Avatar,
  AvatarBadge,
  AvatarFallback,
  AvatarGroup,
  AvatarGroupCount,
  AvatarImage,
  Badge,
  Breadcrumb,
  BreadcrumbEllipsis,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  Icon,
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemFooter,
  ItemGroup,
  ItemHeader,
  ItemMedia,
  ItemSeparator,
  ItemTitle,
  themeTokens,
  useTheme,
} from "@hewel/zgui/components";
import {
  CHECK,
  CHECK_CIRCLE,
  CHEVRON_RIGHT,
  INFO,
  PANEL_LEFT,
  PLUS,
  SEARCH,
} from "@hewel/zgui/icons";
import { DemoRow, Panel } from "../shell.js";

export function CompositesSection(props: { onAction: (label: string) => void }) {
  const [useAsset, setUseAsset] = createSignal(true);
  const [extraLine, setExtraLine] = createSignal(true);
  const [folder, setFolder] = createSignal("Invoices");
  const theme = useTheme();

  return (
    <row class="panel-grid" testId="section-composites">
      <Panel title="Breadcrumb" note="Select an ancestor to navigate back">
        <Breadcrumb testId="composites-breadcrumb">
          <BreadcrumbList>
            {folder() !== "Workspace" && (
              <>
                <BreadcrumbItem>
                  <BreadcrumbLink
                    testId="breadcrumb-workspace"
                    onClick={() => setFolder("Workspace")}
                  >
                    Workspace
                  </BreadcrumbLink>
                </BreadcrumbItem>
                <BreadcrumbSeparator />
                <BreadcrumbItem>
                  <BreadcrumbEllipsis />
                </BreadcrumbItem>
                <BreadcrumbSeparator />
              </>
            )}
            <BreadcrumbItem>
              <BreadcrumbPage testId="breadcrumb-current">{folder()}</BreadcrumbPage>
            </BreadcrumbItem>
          </BreadcrumbList>
        </Breadcrumb>
      </Panel>
      <Panel
        title="Card"
        note="Structured surface with header, action badge, body, and footer actions"
      >
        <Card testId="composites-card">
          <CardHeader>
            <CardTitle>Monthly Settlement</CardTitle>
            <CardDescription>Cycle closes May 28, auto-renew enabled</CardDescription>
            <CardAction>
              <Badge variant="success">Active</Badge>
            </CardAction>
          </CardHeader>
          <CardContent>
            <column class="demo-stack">
              <text>$42.00 for one seat (native bridge + preview runtime).</text>
              <text class="muted-caption">Includes hardware GPU validation and source maps.</text>
            </column>
          </CardContent>
          <CardFooter>
            <Button variant="outline" size="sm" onClick={() => props.onAction("Card: Review")}>
              Review invoice
            </Button>
            <Button size="sm" onClick={() => props.onAction("Card: Approve")}>
              Approve
            </Button>
          </CardFooter>
        </Card>
      </Panel>

      <Panel
        title="Avatar & AvatarGroup"
        note="Bundled PNG asset rendering, fallback initials, status badges, and stacked groups"
      >
        <DemoRow label="Sizes & image fallback">
          <Avatar
            testId="composites-avatar"
            size="sm"
            src={useAsset() ? "assets/avatar.png" : null}
            label="Ada"
          >
            <AvatarFallback>AL</AvatarFallback>
          </Avatar>
          <Avatar size="md" src={useAsset() ? "assets/avatar.png" : null} label="Grace">
            <AvatarFallback>GH</AvatarFallback>
            <AvatarBadge label="Online" />
          </Avatar>
          <Avatar size="lg" label="Verified member">
            <AvatarFallback>JS</AvatarFallback>
            <AvatarImage src={useAsset() ? "assets/avatar.png" : undefined} />
            <AvatarBadge label="Verified">
              <Icon icon={CHECK} size="sm" />
            </AvatarBadge>
          </Avatar>
          <Button
            testId="avatar-toggle-src"
            size="sm"
            variant="outline"
            onClick={() => {
              const next = !useAsset();
              setUseAsset(next);
              props.onAction(next ? "Avatar image on" : "Avatar fallback only");
            }}
          >
            {useAsset() ? "Show initials" : "Load image"}
          </Button>
        </DemoRow>
        <DemoRow label="AvatarGroup">
          <AvatarGroup ariaLabel="Maintainers">
            <Avatar src="assets/avatar.png" label="One">
              <AvatarFallback>A1</AvatarFallback>
            </Avatar>
            <Avatar label="Two">
              <AvatarFallback>GH</AvatarFallback>
            </Avatar>
            <Avatar label="Three">
              <AvatarFallback>BL</AvatarFallback>
            </Avatar>
            <AvatarGroupCount>+5</AvatarGroupCount>
          </AvatarGroup>
          <AvatarGroup size="sm" ariaLabel="Compact team">
            <Avatar size="sm" label="One">
              <AvatarFallback>1</AvatarFallback>
            </Avatar>
            <Avatar size="sm" label="Two">
              <AvatarFallback>2</AvatarFallback>
            </Avatar>
            <AvatarGroupCount>+9</AvatarGroupCount>
          </AvatarGroup>
        </DemoRow>
      </Panel>

      <Panel
        title="Item & ItemGroup"
        note="List row variants (default, outline, muted), header/footer slots, and separators"
      >
        <column class="demo-stack">
          <Item variant="outline">
            <ItemMedia variant="icon">
              <Icon icon={PLUS} size="sm" />
            </ItemMedia>
            <ItemContent>
              <ItemHeader>
                <ItemTitle>Invite contributor</ItemTitle>
                <Badge variant="outline">Team</Badge>
              </ItemHeader>
              <ItemDescription>Link expires in 24 hours after generation.</ItemDescription>
              <ItemFooter>
                <text class="muted-caption">Scoped read/write role</text>
              </ItemFooter>
            </ItemContent>
            <ItemActions>
              <Button size="sm" onClick={() => props.onAction("Item: Invite")}>
                Send
              </Button>
            </ItemActions>
          </Item>

          <Item variant="muted" size="sm">
            <ItemMedia variant="icon">
              <Icon icon={INFO} size="sm" />
            </ItemMedia>
            <ItemContent>
              <ItemTitle>QuickJS stack budget synced</ItemTitle>
              {extraLine() && (
                <ItemDescription>Host updates stack_top at native callbacks.</ItemDescription>
              )}
            </ItemContent>
            <ItemActions>
              <Button size="xs" variant="outline" onClick={() => setExtraLine(value => !value)}>
                {extraLine() ? "Collapse" : "Details"}
              </Button>
            </ItemActions>
          </Item>

          <ItemGroup ariaLabel="Packages">
            <Item size="sm">
              <ItemMedia variant="icon">
                <Icon icon={CHECK_CIRCLE} size="sm" />
              </ItemMedia>
              <ItemContent>
                <ItemTitle>@hewel/zgui</ItemTitle>
              </ItemContent>
              <ItemActions>
                <Badge variant="secondary">Native TSX</Badge>
                <Button size="icon-xs" variant="ghost" ariaLabel="Open package">
                  <Icon icon={CHEVRON_RIGHT} size="sm" />
                </Button>
              </ItemActions>
            </Item>
            <ItemSeparator />
            <Item size="sm">
              <ItemMedia variant="icon">
                <Icon icon={PANEL_LEFT} size="sm" />
              </ItemMedia>
              <ItemContent>
                <ItemTitle>@hewel/zgui-build</ItemTitle>
              </ItemContent>
              <ItemActions>
                <Badge variant="outline">Rolldown</Badge>
              </ItemActions>
            </Item>
          </ItemGroup>
        </column>
      </Panel>

      <Panel
        title="Empty & Theme Context"
        note="Placeholder state pattern and live useTheme() / themeTokens inspection"
      >
        <Empty ariaLabel="Empty search results">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Icon icon={SEARCH} />
            </EmptyMedia>
            <EmptyTitle>No matching snapshots</EmptyTitle>
            <EmptyDescription>Adjust filters or capture a fresh offscreen frame.</EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button
              size="sm"
              variant="outline"
              onClick={() => props.onAction("Empty action clicked")}
            >
              Reset filters
            </Button>
          </EmptyContent>
        </Empty>
        <DemoRow label="Live ThemeContext (useTheme)">
          <Badge variant="outline">
            <text>{`scheme=${theme?.scheme() ?? "none"}`}</text>
          </Badge>
          <Badge variant="outline">
            <text>{`preset=${theme?.light().preset ?? "base"}`}</text>
          </Badge>
          <Badge variant="secondary">
            <text>{`${themeTokens.length} design tokens`}</text>
          </Badge>
        </DemoRow>
      </Panel>
    </row>
  );
}
