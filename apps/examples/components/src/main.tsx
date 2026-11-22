import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  AspectRatio,
  Badge,
  Button,
  ButtonGroup,
  ButtonGroupSeparator,
  ButtonGroupText,
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
  Kbd,
  KbdGroup,
  Separator,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme } from "@hewel/zgui/components";
import { CHECK, INFO, SEARCH } from "@hewel/zgui/icons";
import { Icon } from "@hewel/zgui/components";

const SCHEMES: ColorScheme[] = ["light", "dark", "system"];

function Gallery() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [count, setCount] = createSignal(0);
  const [action, setAction] = createSignal(true);
  const toggleScheme = () =>
    setScheme(current => SCHEMES[(SCHEMES.indexOf(current) + 1) % SCHEMES.length]);

  return (
    <ThemeProvider scheme={scheme()}>
      <scroll class="gallery-scroll">
        <column class="gallery">
          <row class="gallery-row">
            <Button testId="increment" onClick={() => setCount(value => value + 1)}>
              Increment
            </Button>
            <text testId="count">Count: {count()}</text>
            <Button testId="theme-toggle" variant="outline" onClick={toggleScheme}>
              Toggle theme
            </Button>
            <text testId="scheme">{scheme()}</text>
          </row>
          <row class="gallery-row">
            <Button>Default</Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="destructive">Destructive</Button>
            <Button variant="outline">Outline</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="link">Link</Button>
          </row>
          <row class="gallery-row">
            <Button size="xs">Extra small</Button>
            <Button size="sm">Small</Button>
            <Button size="lg">Large</Button>
            <Button size="icon" ariaLabel="Search">
              <Icon icon={SEARCH} />
            </Button>
            <Button size="icon-sm" ariaLabel="Check">
              <Icon icon={CHECK} />
            </Button>
            <Button class="application-override">Override</Button>
          </row>
          <row class="gallery-row">
            <Badge>Default</Badge>
            <Badge variant="secondary">Secondary</Badge>
            <Badge variant="destructive">Destructive</Badge>
            <Badge variant="success">Success</Badge>
            <Badge variant="warning">Warning</Badge>
            <Badge variant="outline">Outline</Badge>
            <Badge variant="ghost">Ghost</Badge>
            <Badge variant="link">Link</Badge>
          </row>
          <row class="gallery-row">
            <Icon icon={INFO} label="Information" size="lg" testId="icon-info" />
            <Icon icon={CHECK} testId="icon-check" />
          </row>
          <Separator />
          <Card testId="card">
            <CardHeader>
              <CardTitle>Build report</CardTitle>
              <CardDescription>Native card slice</CardDescription>
              {action() && (
                <CardAction>
                  <Button
                    size="sm"
                    variant="ghost"
                    testId="card-action-toggle"
                    onClick={() => setAction(false)}
                  >
                    Dismiss
                  </Button>
                </CardAction>
              )}
            </CardHeader>
            <CardContent>
              <text>Card content sits below the header.</text>
            </CardContent>
            <CardFooter>
              <ButtonGroup testId="button-group">
                <Button size="sm">Left</Button>
                <ButtonGroupSeparator />
                <Button size="sm">Right</Button>
                <ButtonGroupText>Text</ButtonGroupText>
              </ButtonGroup>
            </CardFooter>
          </Card>
          <row class="gallery-row">
            <KbdGroup>
              <Kbd>Ctrl</Kbd>
              <Kbd>K</Kbd>
            </KbdGroup>
            <AspectRatio
              ratio={16 / 9}
              style={{ width: "120px", background: "#314665" }}
              testId="aspect"
            />
          </row>
          <ThemeProvider light={{ preset: "ocean" }} dark={{ preset: "ocean" }}>
            <row class="gallery-row" testId="nested-theme">
              <Badge variant="secondary">Ocean nested</Badge>
            </row>
          </ThemeProvider>
        </column>
      </scroll>
    </ThemeProvider>
  );
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Gallery />, root);
}
