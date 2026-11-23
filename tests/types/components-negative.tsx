import {
  Alert, AspectRatio, Avatar, AvatarBadge, AvatarGroup, AvatarImage, Button, ButtonGroup, ButtonGroupSeparator,
  Checkbox, ContextMenuTrigger, EmptyMedia, Icon, Input, InputGroupAddon, InputGroupButton, InputGroupInput,
  InputGroupTextarea, Item, ItemMedia, ItemSeparator, Label, Menubar, MenubarItem, MenubarLabel, MenubarMenu,
  MenubarRadioItem, MenubarSeparator, Progress, Separator, Skeleton, Slider, Switch, Textarea, ThemeProvider, Toggle,
} from "@hewel/zgui/components";
import type { Theme, ThemeToken } from "@hewel/zgui/components";
import { CHECK } from "@hewel/zgui/icons";
import { SliderPrimitive } from "@hewel/zgui/primitives";

const bad = <box>
  {/* @ts-expect-error Button variants are the fixed upstream union. */}
  <Button variant="primary">No</Button>
  {/* @ts-expect-error Button variants are strings, not numbers. */}
  <Button variant={42}>No</Button>
  {/* @ts-expect-error Button sizes are the fixed upstream union. */}
  <Button size="xxl">No</Button>
  {/* @ts-expect-error Roles come from the native role vocabulary. */}
  <Button role="not-a-role">No</Button>
  {/* @ts-expect-error Button groups only accept horizontal or vertical. */}
  <ButtonGroup orientation="diagonal"><Button /></ButtonGroup>
  {/* @ts-expect-error Separators only accept horizontal or vertical. */}
  <Separator orientation="diagonal" />
  {/* @ts-expect-error Decorative is a boolean. */}
  <Separator decorative="yes" />
  {/* @ts-expect-error Aspect ratios are numbers. */}
  <AspectRatio ratio="16:9" />
  {/* @ts-expect-error Aspect ratios are numbers. */}
  <AspectRatio ratio={null} />
  {/* @ts-expect-error Color schemes are system, light, or dark. */}
  <ThemeProvider scheme="auto"><box /></ThemeProvider>
  {/* @ts-expect-error Themes are preset/token objects, not strings. */}
  <ThemeProvider light="ocean"><box /></ThemeProvider>
  {/* @ts-expect-error ThemeProvider has no tokens prop. */}
  <ThemeProvider tokens={{}}><box /></ThemeProvider>
  {/* @ts-expect-error Icons take IconData, not names. */}
  <Icon icon="check" />
  {/* @ts-expect-error Icon sizes are the fixed upstream union. */}
  <Icon icon={CHECK} size="md2" />
  {/* @ts-expect-error Icon labels are strings. */}
  <Icon icon={CHECK} label={4} />
  {/* @ts-expect-error Icons are childless replaced content. */}
  <Icon icon={CHECK}>content</Icon>
  {/* @ts-expect-error The path data is owned by the icon value. */}
  <Icon icon={CHECK} d="M0 0" />
  {/* @ts-expect-error The view box is owned by the icon value. */}
  <Icon icon={CHECK} viewBox="0 0 24 24" />
  {/* @ts-expect-error Separators are childless. */}
  <Separator>content</Separator>
  {/* @ts-expect-error Group separators are childless. */}
  <ButtonGroupSeparator>content</ButtonGroupSeparator>
  {/* @ts-expect-error Separator props expose only the component orientation name. */}
  <Separator orientationNative="x" />
  {/* @ts-expect-error Checkbox states are boolean or mixed. */}
  <Checkbox checked="half" />
  {/* @ts-expect-error Switch states are boolean only. */}
  <Switch checked="mixed" />
  {/* @ts-expect-error Checkboxes are childless. */}
  <Checkbox>content</Checkbox>
  {/* @ts-expect-error Inputs are childless; their content is native-owned. */}
  <Input>content</Input>
  {/* @ts-expect-error Secure is a strict boolean attribute. */}
  <Input secure="yes" />
  {/* @ts-expect-error The editor element has no secure attribute. */}
  <Textarea secure />
  {/* @ts-expect-error Only field elements can be secure. */}
  <editor secure />
  {/* @ts-expect-error Ordinary elements cannot be secure. */}
  <box secure />
  {/* @ts-expect-error onValueChange receives the edited string. */}
  <Input onValueChange={(value: number) => value} />
  {/* @ts-expect-error onCheckedChange receives the checked state. */}
  <Checkbox onCheckedChange={(value: number) => value} />
  {/* @ts-expect-error Relations are native nodes or arrays of them, never IDs. */}
  <box ariaLabelledBy={3} />
  {/* @ts-expect-error Relations are native nodes or arrays of them, never IDs. */}
  <box ariaDescribedBy="label-1" />
  {/* @ts-expect-error Label controls resolve to native nodes. */}
  <Label control={3}>No</Label>
  {/* @ts-expect-error Activation claims exist only on pointer down. */}
  <box onPointerUp={event => event.activateNow()} />
  {/* @ts-expect-error Activation claims exist only on pointer down. */}
  <box onKeyDown={event => event.activateNow()} />
  {/* @ts-expect-error Toggle pressed state is boolean. */}
  <Toggle pressed="yes" />
  {/* @ts-expect-error Toggle variants are default or outline. */}
  <Toggle variant="primary" />
  {/* @ts-expect-error Alert variants are default or destructive. */}
  <Alert variant="warning" />
  {/* @ts-expect-error Progress values are numbers. */}
  <Progress value="50%" />
  {/* @ts-expect-error Skeletons are childless. */}
  <Skeleton>content</Skeleton>
  {/* @ts-expect-error Live values come from the native vocabulary. */}
  <box ariaLive="loud" />
  {/* @ts-expect-error Numeric accessibility values are numbers. */}
  <box ariaValueNow="50" />
  {/* @ts-expect-error Placeholders are strings. */}
  <field placeholder={3} />
  {/* @ts-expect-error Placeholders are strings. */}
  <editor placeholder={3} />
  {/* @ts-expect-error Avatar sizes are the fixed upstream union. */}
  <Avatar size="xl" />
  {/* @ts-expect-error Avatar images are childless replaced content. */}
  <AvatarImage src="a.png">content</AvatarImage>
  {/* @ts-expect-error Avatar badge labels are strings. */}
  <AvatarBadge label={4} />
  {/* @ts-expect-error Avatar group sizes are the fixed upstream union. */}
  <AvatarGroup size="xl" />
  {/* @ts-expect-error Empty media variants are default or icon. */}
  <EmptyMedia variant="tile" />
  {/* @ts-expect-error Item variants are the fixed upstream union. */}
  <Item variant="solid" />
  {/* @ts-expect-error Item sizes are md or sm. */}
  <Item size="lg" />
  {/* @ts-expect-error Item media variants are the fixed upstream union. */}
  <ItemMedia variant="photo" />
  {/* @ts-expect-error Item separators are childless. */}
  <ItemSeparator>content</ItemSeparator>
  {/* @ts-expect-error Addon alignment comes from the fixed upstream union. */}
  <InputGroupAddon align="left" />
  {/* @ts-expect-error Group buttons accept only the compact sizes. */}
  <InputGroupButton size="lg" />
  {/* @ts-expect-error Group buttons accept only the compact sizes. */}
  <InputGroupButton size="icon-lg" />
  {/* @ts-expect-error Textareas cannot be secure. */}
  <InputGroupTextarea secure />
  {/* @ts-expect-error onValueChange receives the edited string. */}
  <InputGroupInput onValueChange={(value: number) => value} />
  {/* @ts-expect-error Menubar labels are strings. */}
  <Menubar label={4} />
  {/* @ts-expect-error MenubarMenu requires a value. */}
  <MenubarMenu />
  {/* @ts-expect-error MenubarRadioItem requires a value. */}
  <MenubarRadioItem />
  {/* @ts-expect-error MenubarItem does not accept inset styling. */}
  <MenubarItem inset>No</MenubarItem>
  {/* @ts-expect-error MenubarLabel does not accept inset styling. */}
  <MenubarLabel inset>No</MenubarLabel>
  {/* @ts-expect-error Menubar separators are childless. */}
  <MenubarSeparator>content</MenubarSeparator>
  {/* @ts-expect-error ContextMenuTrigger onContextMenu cannot capture the pointer. */}
  <ContextMenuTrigger onContextMenu={event => event.capturePointer()} />
  {/* @ts-expect-error Styled Slider forbids children. */}
  <Slider><box /></Slider>
  {/* @ts-expect-error SliderPrimitive is horizontal-only. */}
  <SliderPrimitive orientation="vertical" />
  {/* @ts-expect-error SliderPrimitive pointerStep must be a number. */}
  <SliderPrimitive pointerStep="1" />
  {/* @ts-expect-error SliderPrimitive onDraggingChange receives a boolean. */}
  <SliderPrimitive onDraggingChange={(dragging: string) => dragging} />
  {/* @ts-expect-error SliderPrimitive render state exposes numeric value accessor. */}
  <SliderPrimitive>{({ value }) => { const badVal: string = value(); return <text>{badVal}</text>; }}</SliderPrimitive>
</box>;

// @ts-expect-error Presets come from the pinned upstream set.
const badTheme: Theme = { preset: "neptune" };
// @ts-expect-error Token keys come from the pinned upstream property list.
const badTokens: Theme = { tokens: { "--zui-made-up": "red" } };
// @ts-expect-error Token overrides are strings.
const tokenValue: Partial<Record<ThemeToken, string>> = { "--zui-color-primary": 4 };

void bad;
void badTheme;
void badTokens;
void tokenValue;
