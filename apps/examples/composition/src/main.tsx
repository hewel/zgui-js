import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Avatar,
  AvatarBadge,
  AvatarFallback,
  AvatarGroup,
  AvatarGroupCount,
  AvatarImage,
  Button,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  Icon,
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
  InputGroupTextarea,
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemSeparator,
  ItemTitle,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme } from "@hewel/zgui/components";
import { CHECK, SEARCH } from "@hewel/zgui/icons";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Panel />, root);
}

function Panel() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [avatarSrc, setAvatarSrc] = createSignal<string | null>("assets/red.png");
  const [descCount, setDescCount] = createSignal(2);
  const [query, setQuery] = createSignal("");
  const [actions, setActions] = createSignal(0);
  const [invalid, setInvalid] = createSignal(false);

  return (
    <ThemeProvider scheme={scheme()}>
      <scroll class="panel-scroll">
        <row class="panel">
          <column class="panel-column">
            <row class="panel-row">
              <Avatar testId="avatar" src={avatarSrc()} label="Zora">
                <AvatarFallback testId="avatar-fallback">ZR</AvatarFallback>
                <AvatarBadge testId="avatar-badge" />
              </Avatar>
              <Avatar testId="avatar-failed" src="assets/broken.png" label="Broken">
                <AvatarFallback testId="avatar-failed-fallback">FB</AvatarFallback>
              </Avatar>
              <Avatar label="Composed">
                <AvatarFallback>EX</AvatarFallback>
                <AvatarImage src="assets/red.png" />
              </Avatar>
              <AvatarGroup size="sm" ariaLabel="Team">
                <Avatar size="sm" src="assets/red.png" label="One">
                  <AvatarFallback>1</AvatarFallback>
                </Avatar>
                <Avatar size="sm" label="Two">
                  <AvatarFallback>2</AvatarFallback>
                </Avatar>
                <AvatarGroupCount>+3</AvatarGroupCount>
              </AvatarGroup>
            </row>
            <row class="panel-row">
              <Button
                testId="avatar-source"
                size="sm"
                variant="outline"
                onClick={() => setAvatarSrc(value => (value == null ? "assets/red.png" : null))}
              >
                Source
              </Button>
              <Button
                testId="composition-theme"
                size="sm"
                variant="outline"
                onClick={() => setScheme(value => (value === "light" ? "dark" : "light"))}
              >
                Theme
              </Button>
            </row>
            <Empty ariaLabel="Empty state">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Icon icon={SEARCH} />
                </EmptyMedia>
                <EmptyTitle>No results</EmptyTitle>
                <EmptyDescription>Adjust your filters and try again.</EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button
                  testId="empty-reset"
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setQuery("");
                    setInvalid(false);
                  }}
                >
                  Clear filters
                </Button>
              </EmptyContent>
            </Empty>
            <Item testId="item" variant="outline">
              <ItemMedia variant="icon">
                <Icon icon={CHECK} />
              </ItemMedia>
              <ItemContent>
                <ItemTitle>Document</ItemTitle>
                {descCount() >= 1 && (
                  <ItemDescription testId="item-description-a">Synced an hour ago.</ItemDescription>
                )}
                {descCount() >= 2 && (
                  <ItemDescription testId="item-description-b">
                    Shared with two people.
                  </ItemDescription>
                )}
                <ItemActions>
                  <Button size="xs" variant="ghost">
                    Open
                  </Button>
                </ItemActions>
              </ItemContent>
            </Item>
            <row class="panel-row">
              <Button
                testId="item-descriptions"
                size="sm"
                variant="outline"
                onClick={() => setDescCount(count => (count + 2) % 3)}
              >
                Descriptions
              </Button>
              <text testId="item-description-count">{descCount()}</text>
            </row>
          </column>
          <column class="panel-column">
            <InputGroup testId="group" invalid={invalid()}>
              <InputGroupAddon>
                <Icon icon={SEARCH} />
              </InputGroupAddon>
              <InputGroupInput
                testId="grouped-input"
                label="Search"
                value={query()}
                onValueChange={setQuery}
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton
                  testId="grouped-action"
                  onClick={() => setActions(count => count + 1)}
                >
                  Go
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
            <row class="panel-row">
              <text testId="query-value">{query()}</text>
              <text testId="query-actions">{actions()}</text>
            </row>
            <InputGroup>
              <InputGroupTextarea testId="grouped-area" defaultValue="Notes" readOnly />
              <InputGroupAddon align="block-end">
                <InputGroupText>Read only</InputGroupText>
              </InputGroupAddon>
            </InputGroup>
            <Button
              testId="frame-toggle"
              size="sm"
              variant="outline"
              onClick={() => setInvalid(value => !value)}
            >
              Frame
            </Button>
            <ItemGroup ariaLabel="Files">
              <Item>
                <ItemContent>
                  <ItemTitle>First</ItemTitle>
                </ItemContent>
              </Item>
              <ItemSeparator />
              <Item>
                <ItemContent>
                  <ItemTitle>Second</ItemTitle>
                </ItemContent>
              </Item>
            </ItemGroup>
          </column>
        </row>
      </scroll>
    </ThemeProvider>
  );
}
