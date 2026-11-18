import { createContext, createSignal, useContext } from "@hewel/zgui";
import type { Context, NativeChild } from "@hewel/zgui";
import {
  Accordion, AccordionContent, AccordionItem, AccordionTrigger,
  Alert, AlertDescription, AlertTitle, AspectRatio, Avatar, AvatarBadge,
  AvatarFallback, AvatarGroup, AvatarGroupCount, AvatarImage, Badge, Button,
  ButtonGroup, ButtonGroupSeparator, ButtonGroupText, Card, CardAction,
  CardContent, CardDescription, CardFooter, CardHeader, CardTitle, Checkbox,
  Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle,
  Icon, Input, InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput,
  InputGroupText, InputGroupTextarea, Item, ItemActions, ItemContent,
  ItemDescription, ItemFooter, ItemGroup, ItemHeader, ItemMedia, ItemSeparator,
  ItemTitle, Kbd, KbdGroup, Label, Progress, RadioGroup, RadioGroupItem,
  Collapsible, CollapsibleContent, CollapsibleTrigger,
  Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle,
  DialogDescription, DialogFooter, DialogClose, DialogDismiss,
  Popover, PopoverTrigger, PopoverContent, PopoverHeader, PopoverTitle,
  PopoverDescription, PopoverClose,
  Tooltip, TooltipProvider, TooltipTrigger, TooltipContent, TooltipArrow,
  HoverCard, HoverCardTrigger, HoverCardContent,
  DropdownMenu, DropdownMenuTrigger, DropdownMenuContent,
  MenuContent, MenuTypeahead, MenuItem, MenuCheckboxItem, MenuRadioGroup, MenuRadioItem,
  MenuLabel, MenuGroup, MenuSeparator, MenuShortcut,
  MenuSub, MenuSubTrigger, MenuSubContent, headingToward,
  ContextMenu, ContextMenuTrigger, ContextMenuContent, useContextMenuPlace,
  Menubar, MenubarMenu, MenubarTrigger, MenubarContent, MenubarArrows,
  MenubarItem, MenubarCheckboxItem, MenubarRadioItem, MenubarLabel,
  MenubarSeparator, MenubarShortcut, MenubarGroup, MenubarRadioGroup,
  Separator, Skeleton, Slider, Spinner, Switch, Tabs, TabsContent, TabsList, TabsTrigger,
  Textarea, ThemeProvider, Toggle, ToggleGroup, ToggleGroupItem, themeTokens, useTheme,
} from "@hewel/zgui/components";
import type {
  AccordionContentProps, AccordionItemProps, AccordionProps, AccordionSelection,
  AccordionTriggerProps, AspectRatioProps, AvatarProps, AvatarSize, BadgeProps,
  BadgeVariant, ButtonProps,
  ButtonSize, ButtonVariant, CheckedState, CheckboxProps, ColorScheme,
  EmptyMediaProps, InputGroupAddonAlign, InputGroupButtonSize, InputGroupProps,
  InputProps, ItemMediaVariant, ItemProps, ItemSize, ItemVariant, LabelProps,
  GroupOrientation, ProgressProps, RadioGroupItemProps, RadioGroupProps, SliderProps,
  CollapsibleContentProps, CollapsibleProps, CollapsibleTriggerProps,
  DialogProps, DialogTriggerProps, DialogContentProps, DialogHeaderProps,
  DialogTitleProps, DialogDescriptionProps, DialogFooterProps, DialogCloseProps, DialogDismissProps,
  PopoverProps, PopoverTriggerProps, PopoverContentProps, PopoverHeaderProps,
  PopoverTitleProps, PopoverDescriptionProps, PopoverCloseProps,
  TooltipProps, TooltipProviderProps, TooltipTriggerProps, TooltipContentProps, TooltipArrowProps,
  HoverCardProps, HoverCardTriggerProps, HoverCardContentProps,
  DropdownMenuProps, DropdownMenuTriggerProps, DropdownMenuContentProps,
  MenuContentProps, MenuTypeaheadProps, MenuItemProps, MenuCheckboxItemProps,
  MenuRadioGroupProps, MenuRadioItemProps, MenuLabelProps, MenuGroupProps,
  MenuSeparatorProps, MenuShortcutProps,
  MenuSubProps, MenuSubTriggerProps, MenuSubContentProps,
  ContextMenuProps, ContextMenuTriggerProps, ContextMenuContentProps, ContextMenuPlace, ContextMenuPoint,
  MenubarProps, MenubarMenuProps, MenubarTriggerProps, MenubarContentProps, MenubarArrowsProps, MenubarBarControl,
  MenubarItemProps, MenubarCheckboxItemProps, MenubarRadioItemProps, MenubarLabelProps,
  MenubarSeparatorProps, MenubarShortcutProps, MenubarGroupProps, MenubarRadioGroupProps,
  PopperSide, PopperAlign, PopperPlacement,
  SwitchProps, Theme, ThemeContext, ThemePreset, ThemeToken, ToggleGroupItemProps,
  TabsActivation, TabsContentProps, TabsListProps, TabsListVariant, TabsProps, TabsTriggerProps,
  ToggleGroupProps, ToggleProps, ToggleSelection,
} from "@hewel/zgui/components";
import { CHECK, CHEVRON_DOWN, Icon as IconComponent, SEARCH } from "@hewel/zgui/icons";
import type { IconData, IconProps, IconSize } from "@hewel/zgui/icons";
import type { NativeNode } from "@hewel/zgui";

const scheme: ColorScheme = "system";
const preset: ThemePreset = "ocean";
const token: ThemeToken = themeTokens[0];
const light: Theme = { preset, tokens: { [token]: "#102030" } };
const dark: Theme = { preset: "midnight" };
const variants: ButtonVariant[] = ["default", "secondary", "destructive", "outline", "ghost", "link"];
const sizes: ButtonSize[] = ["xs", "sm", "md", "lg", "icon", "icon-xs", "icon-sm", "icon-lg"];
const badgeVariants: BadgeVariant[] = ["default", "secondary", "destructive", "success", "warning", "outline", "ghost", "link"];
const iconSizes: IconSize[] = ["sm", "md", "lg", "xl"];
const [signal] = createSignal<ColorScheme>("light");

const ThemeSlot: Context<Theme | undefined> = createContext<Theme | undefined>(undefined);
const themed: NativeChild = <ThemeSlot value={light}><box /></ThemeSlot>;

function Themed() {
  const provided = useContext(ThemeSlot);
  const current = useTheme();
  const accessors: ThemeContext | undefined = current;
  const active: ColorScheme | undefined = accessors?.scheme();
  const resolved: Theme | undefined = accessors?.light();
  return <text>{provided?.preset}{active}{resolved?.preset}</text>;
}

const icon: IconData = CHECK;
const namedIcon: IconProps = { icon: SEARCH, size: "lg", label: "Search", "data-kind": "nav" };
const buttonProps: ButtonProps = { variant: "outline", size: "sm", disabled: false, "data-state": "idle" };
const badgeProps: BadgeProps = { variant: "success", title: "State" };
const ratioProps: AspectRatioProps = { ratio: 16 / 9, class: "wide" };
const theIcon: typeof IconComponent = Icon;

const gallery = <ThemeProvider scheme={signal()} light={light} dark={dark}>
  <Themed />
  <row>
    <Button {...buttonProps} onClick={event => event.preventDefault()}>Save</Button>
    <Button variant={variants[1]} size={sizes[6]}><Icon icon={CHECK} /></Button>
    <Button variant="link" size="icon" ariaLabel="More"><Icon icon={CHEVRON_DOWN} /></Button>
    <Button role="tab" selected>Tab</Button>
    <Badge {...badgeProps}>Stable</Badge>
    <Icon {...namedIcon} />
    <Icon icon={icon} testId="decorative" />
  </row>
  <Card role="region" ariaLabel="Report">
    <CardHeader>
      <CardTitle>Monthly</CardTitle>
      <CardDescription>Summary</CardDescription>
      <CardAction><Button size="sm" variant="ghost">Close</Button></CardAction>
    </CardHeader>
    <CardContent><AspectRatio {...ratioProps}><box /></AspectRatio></CardContent>
    <CardFooter>
      <ButtonGroup orientation="vertical" ariaLabel="Actions">
        <Button size="xs">One</Button>
        <ButtonGroupSeparator />
        <ButtonGroupText>Two more</ButtonGroupText>
      </ButtonGroup>
    </CardFooter>
  </Card>
  <Separator orientation="horizontal" decorative={false} />
  <Separator decorative />
  <KbdGroup><Kbd>Ctrl</Kbd><Kbd>K</Kbd></KbdGroup>
</ThemeProvider>;

let fieldRef: NativeNode | undefined;
let labelRef: NativeNode | undefined;
const checked: CheckedState = "mixed";
const checkboxProps: CheckboxProps = { checked, onCheckedChange: next => next === "mixed", invalid: true };
const switchProps: SwitchProps = { checked: true, size: "sm", onCheckedChange: next => !next };
const toggleProps: ToggleProps = { pressed: false, variant: "outline", size: "lg", label: "Bold" };
const groupAxis: GroupOrientation = "both";
const toggleSelection: ToggleSelection = "multiple";
const radioProps: RadioGroupProps = { defaultValue: "b", orientation: groupAxis, label: "Fruit" };
const radioItemProps: RadioGroupItemProps = { value: "a", label: "Apple", disabled: false };
const toggleGroupProps: ToggleGroupProps = {
  selection: toggleSelection, variant: "outline", size: "sm", spacing: 0,
  value: ["a", "b"] as readonly string[],
  onValueChange: value => { const next: readonly string[] = value; void next; },
};
const toggleItemProps: ToggleGroupItemProps = { value: "a", variant: "default", size: "lg" };
const tabsActivation: TabsActivation = "manual";
const tabsListVariant: TabsListVariant = "line";
const tabsProps: TabsProps = {
  defaultValue: "a", orientation: "vertical", activation: tabsActivation, label: "Pages",
  onValueChange: value => { const next: string = value; void next; },
};
const tabsListProps: TabsListProps = { variant: tabsListVariant, label: "Pages" };
const tabsTriggerProps: TabsTriggerProps = { value: "a", disabled: true };
const tabsContentProps: TabsContentProps = { value: "a", keepMounted: true };
const sliderProps: SliderProps = {
  min: -0.5, max: 0.5, step: 0.25, defaultValue: 0, orientation: "horizontal",
  label: "Balance", onValueChange: value => { const next: number = value; void next; },
};
const selection: AccordionSelection = "multiple";
const openValues: readonly string[] = ["b", "c"];
const collapsibleProps: CollapsibleProps = {
  defaultOpen: true, disabled: false,
  onOpenChange: next => { const open: boolean = next; void open; },
};
const collapsibleTriggerProps: CollapsibleTriggerProps = { disabled: false, "data-state": "open" };
const collapsibleContentProps: CollapsibleContentProps = { "--zui-collapsible-height": "24px" };
const accordionProps: AccordionProps = {
  value: openValues, selection, collapsible: false, disabled: false,
  onValueChange: values => { const next: readonly string[] = values; void next; },
};
const accordionItemProps: AccordionItemProps = { value: "b", disabled: false };
const accordionTriggerProps: AccordionTriggerProps = { level: 2 };
const accordionContentProps: AccordionContentProps = { role: "region" };
const inputProps: InputProps = { label: "Name", placeholder: "Ada", secure: true, required: true };
const labelProps: LabelProps = { control: () => fieldRef, disabled: false };
const progressProps: ProgressProps = { value: 25, max: 200, label: "Loading" };

const controls = <column>
  <Label {...labelProps} ref={node => { labelRef = node; }}>Name</Label>
  <Input {...inputProps} ref={node => { fieldRef = node; }}
    ariaLabelledBy={labelRef ?? null} value="Ada"
    onValueChange={value => { void value.length; }} />
  <Input placeholder={null} ariaControls={labelRef ? [labelRef] : null} ariaDescribedBy={null} />
  <Textarea label="Notes" defaultValue="Read only" readOnly
    onValueChange={value => { void value; }} />
  <Checkbox {...checkboxProps} defaultChecked="mixed" ariaBusy={false}
    onPointerDown={event => event.activateNow()} />
  <Switch {...switchProps} />
  <Toggle {...toggleProps} defaultPressed={false} onPressedChange={next => { void next; }}>B</Toggle>
  <Toggle ariaLabel="Italic">I</Toggle>
  <RadioGroup {...radioProps} onValueChange={value => { void value.length; }}>
    <RadioGroupItem {...radioItemProps} />
    <RadioGroupItem value="b" />
  </RadioGroup>
  <Slider {...sliderProps} />
  <Slider value={40} min={0} max={100} step={5} ariaLabelledBy={null} />
  <ToggleGroup {...toggleGroupProps}>
    <ToggleGroupItem {...toggleItemProps}>A</ToggleGroupItem>
    <ToggleGroupItem value="b" ariaLabel="B">B</ToggleGroupItem>
  </ToggleGroup>
  <Progress {...progressProps} />
  <Progress value={null} />
  <Alert variant="destructive" live={false} icon={false}>
    <AlertTitle>Title</AlertTitle>
    <AlertDescription>Body</AlertDescription>
  </Alert>
  <Alert live icon><AlertTitle>Default</AlertTitle></Alert>
  <Spinner label="Working" />
  <Spinner />
  <Skeleton ariaHidden />
  <field ariaBusy ariaLive="assertive" ariaValueMin={0} ariaValueMax={1} ariaValueNow={0.5} ariaValueStep={0.1}
    ariaOwns={fieldRef ?? null} />
</column>;

const avatarProps: AvatarProps = { src: "a.png", size: "lg" as AvatarSize, label: "Ada" };
const mediaProps: EmptyMediaProps = { variant: "icon" };
const align: InputGroupAddonAlign = "block-end";
const groupButtonSize: InputGroupButtonSize = "icon-sm";
const groupProps: InputGroupProps = { "data-kind": "search" };
const itemProps: ItemProps = { variant: "muted" as ItemVariant, size: "sm" as ItemSize };
const itemMedia: ItemMediaVariant = "image";

const composed = <column>
  <Avatar {...avatarProps}>
    <AvatarFallback>AL</AvatarFallback>
    <AvatarBadge label="Online"><Icon icon={CHECK} /></AvatarBadge>
  </Avatar>
  <Avatar src={null} ariaHidden={false} ariaLabel="Override" />
  <Avatar label="Solo" size="sm"><AvatarImage src="b.png" /></Avatar>
  <AvatarGroup size="sm" ariaLabel="Team">
    <Avatar label="One" /><AvatarGroupCount>+2</AvatarGroupCount>
  </AvatarGroup>
  <Empty role="region" ariaLabel="Empty state">
    <EmptyHeader>
      <EmptyMedia {...mediaProps}><Icon icon={SEARCH} /></EmptyMedia>
      <EmptyMedia />
      <EmptyTitle>No results</EmptyTitle>
      <EmptyDescription>Try another query</EmptyDescription>
    </EmptyHeader>
    <EmptyContent><Button size="sm">Retry</Button></EmptyContent>
  </Empty>
  <Item {...itemProps}>
    <ItemMedia variant="icon"><Icon icon={CHECK} /></ItemMedia>
    <ItemMedia variant={itemMedia} />
    <ItemContent>
      <ItemHeader><ItemTitle>Title</ItemTitle></ItemHeader>
      <ItemDescription>First</ItemDescription>
      <ItemDescription>Second</ItemDescription>
      <ItemFooter><ItemActions><Button size="xs">Go</Button></ItemActions></ItemFooter>
    </ItemContent>
  </Item>
  <ItemGroup ariaLabel="List">
    <Item variant="outline" />
    <ItemSeparator />
  </ItemGroup>
  <InputGroup {...groupProps} disabled invalid>
    <InputGroupAddon align={align}><InputGroupText>@</InputGroupText></InputGroupAddon>
    <InputGroupAddon />
    <InputGroupInput secure={false} label="Search" onValueChange={v => { void v; }} />
    <InputGroupInput secure />
    <InputGroupTextarea readOnly defaultValue="Notes" />
    <InputGroupButton size={groupButtonSize}>Go</InputGroupButton>
    <InputGroupButton size="xs" variant="outline" />
  </InputGroup>
  <Tabs {...tabsProps}>
    <TabsList {...tabsListProps}>
      <TabsTrigger {...tabsTriggerProps} />
      <TabsTrigger value="b" />
    </TabsList>
    <TabsContent {...tabsContentProps} />
    <TabsContent value="b" />
  </Tabs>
  <Collapsible {...collapsibleProps}>
    <CollapsibleTrigger {...collapsibleTriggerProps}>Details</CollapsibleTrigger>
    <CollapsibleContent {...collapsibleContentProps}><text>Body</text></CollapsibleContent>
  </Collapsible>
  <box role="heading" ariaLevel={7}>Native heading</box>
  <Accordion {...accordionProps}>
    <AccordionItem {...accordionItemProps}>
      <AccordionTrigger {...accordionTriggerProps}>Second</AccordionTrigger>
      <AccordionContent {...accordionContentProps}><text>Two</text></AccordionContent>
    </AccordionItem>
  </Accordion>
</column>;

const [dialogOpen, setDialogOpen] = createSignal(false);
let surfaceRef: NativeNode | undefined;
let triggerRef: NativeNode | undefined;
const dialogProps: DialogProps = { open: dialogOpen(), onOpenChange: next => setDialogOpen(next) };
const dialogTriggerProps: DialogTriggerProps = { variant: "secondary", size: "lg" };
const dialogContentProps: DialogContentProps = {
  dismissOnOutsidePress: false, dismissOnEscape: true, dismissControl: true,
  onAnimationEnd: event => console.log(event.animationName, event.target),
};
const dialogHeaderProps: DialogHeaderProps = { role: "group" };
const dialogTitleProps: DialogTitleProps = { id: "dialog-title" };
const dialogDescriptionProps: DialogDescriptionProps = { class: "muted" };
const dialogFooterProps: DialogFooterProps = { closeControl: true };
const dialogCloseProps: DialogCloseProps = { variant: "outline" };
const dialogDismissProps: DialogDismissProps = { ariaLabel: "Close dialog" };

const dialogs = <Dialog {...dialogProps}>
  <DialogTrigger {...dialogTriggerProps} ref={node => { triggerRef = node; }}>Rename</DialogTrigger>
  <DialogTrigger variant="link" ariaHasPopup="dialog" ariaPopupFor={surfaceRef ?? null}>Second</DialogTrigger>
  <DialogContent {...dialogContentProps} ref={node => { surfaceRef = node; }}>
    <DialogHeader {...dialogHeaderProps}>
      <DialogTitle {...dialogTitleProps}>Rename project</DialogTitle>
      <DialogDescription {...dialogDescriptionProps}>Choose a new name.</DialogDescription>
    </DialogHeader>
    <Input label="Name" placeholder="Ada" />
    <DialogFooter {...dialogFooterProps}>
      <DialogClose {...dialogCloseProps}>Cancel</DialogClose>
    </DialogFooter>
    <DialogDismiss {...dialogDismissProps} />
  </DialogContent>
  <box ariaHasPopup="menu" ariaPopupFor={triggerRef ?? null} />
  <box ariaHasPopup={null} ariaPopupFor={null} />
</Dialog>;

const [popoverOpen, setPopoverOpen] = createSignal(false);
const popoverSide: PopperSide = "right";
const popoverAlign: PopperAlign = "end";
const popoverPlacement: PopperPlacement = { side: popoverSide, align: popoverAlign };
const popoverProps: PopoverProps = { open: popoverOpen(), onOpenChange: next => setPopoverOpen(next) };
const popoverTriggerProps: PopoverTriggerProps = { variant: "outline", size: "sm" };
const popoverContentProps: PopoverContentProps = {
  placement: popoverPlacement, dismissOnOutsidePress: true, dismissOnEscape: false,
  onTransitionEnd: event => console.log(event.target),
};
const popoverHeaderProps: PopoverHeaderProps = { role: "group" };
const popoverTitleProps: PopoverTitleProps = { id: "popover-title" };
const popoverDescriptionProps: PopoverDescriptionProps = { class: "muted" };
const popoverCloseProps: PopoverCloseProps = { variant: "secondary" };

const popovers = <Popover {...popoverProps}>
  <PopoverTrigger {...popoverTriggerProps}>Share</PopoverTrigger>
  <PopoverContent {...popoverContentProps} placement={{ side: "bottom", align: "start" }} offset={6}>
    <PopoverHeader {...popoverHeaderProps}>
      <PopoverTitle {...popoverTitleProps}>Share project</PopoverTitle>
      <PopoverDescription {...popoverDescriptionProps}>Send a link.</PopoverDescription>
    </PopoverHeader>
    <Input label="Email" placeholder="ada@example.com" />
    <PopoverClose {...popoverCloseProps}>Done</PopoverClose>
  </PopoverContent>
</Popover>;

const [tooltipOpen, setTooltipOpen] = createSignal(false);
const tooltipProps: TooltipProps = {
  open: tooltipOpen(), onOpenChange: next => setTooltipOpen(next), delay: 120, closeDelay: 200,
};
const tooltipProviderProps: TooltipProviderProps = { delay: 300, closeDelay: 100 };
const tooltipTriggerProps: TooltipTriggerProps = { class: "hint", onFocus: event => { console.log(event.cause); } };
const tooltipContentProps: TooltipContentProps = {
  arrow: false, placement: { side: "bottom", align: "start" }, offset: 8,
  onPointerEnter: event => { console.log(event.position.x, event.pointerKind); },
};
const tooltipArrowProps: TooltipArrowProps = { class: "arrow" };
const hoverCardProps: HoverCardProps = { delay: 500, closeDelay: 150 };
const hoverCardTriggerProps: HoverCardTriggerProps = { onBlur: event => { console.log(event.cause); } };
const hoverCardContentProps: HoverCardContentProps = { placement: { side: "right", align: "center" }, offset: 12 };

const tooltips = <TooltipProvider {...tooltipProviderProps}>
  <Tooltip {...tooltipProps}>
    <TooltipTrigger {...tooltipTriggerProps}>Hint</TooltipTrigger>
    <TooltipContent {...tooltipContentProps}>
      Copied to clipboard
      <TooltipArrow {...tooltipArrowProps} />
    </TooltipContent>
  </Tooltip>
  <Tooltip defaultOpen>
    <TooltipTrigger>Default hint</TooltipTrigger>
    <TooltipContent>Instant</TooltipContent>
  </Tooltip>
</TooltipProvider>;
type AssertArrow<T extends true> = T;
export type ArrowHasNoChildren = AssertArrow<Exclude<TooltipArrowProps["children"], undefined> extends never ? true : false>;
const hoverCards = <HoverCard {...hoverCardProps}>
  <HoverCardTrigger {...hoverCardTriggerProps}>Profile</HoverCardTrigger>
  <HoverCardContent {...hoverCardContentProps}>Profile preview</HoverCardContent>
</HoverCard>;

const [menuOpen, setMenuOpen] = createSignal(false);
const [checkState, setCheckState] = createSignal(true);
const [radioPick, setRadioPick] = createSignal("x");
const dropdownMenuProps: DropdownMenuProps = { open: menuOpen(), onOpenChange: next => setMenuOpen(next) };
const dropdownMenuTriggerProps: DropdownMenuTriggerProps = { variant: "outline", disabled: false };
const dropdownMenuContentProps: DropdownMenuContentProps = {
  placement: { side: "top", align: "end" }, offset: 8, dismissOnOutsidePress: true,
};
const menuContentProps: MenuContentProps = { placement: { side: "right", align: "start" } };
const menuTypeaheadProps: MenuTypeaheadProps = { class: "keys" };
const menuItemProps: MenuItemProps = {
  destructive: true, inset: true, closeOnSelect: false, shortcut: "Ctrl+P",
  onSelect: () => { void checkState; },
};
const menuCheckboxItemProps: MenuCheckboxItemProps = {
  checked: checkState(), defaultChecked: false, closeOnSelect: false,
  onCheckedChange: next => setCheckState(next),
};
const menuRadioGroupProps: MenuRadioGroupProps = {
  label: "Tone", value: radioPick(), defaultValue: "x",
  onValueChange: next => setRadioPick(next),
};
const menuRadioItemProps: MenuRadioItemProps = { value: "y", closeOnSelect: false };
const menuLabelProps: MenuLabelProps = { inset: true };
const menuGroupProps: MenuGroupProps = { label: "File" };
const menuSeparatorProps: MenuSeparatorProps = { class: "gap" };
const menuShortcutProps: MenuShortcutProps = { class: "keys" };
const menuSubProps: MenuSubProps = { open: menuOpen(), defaultOpen: false, onOpenChange: next => setMenuOpen(next) };
const menuSubTriggerProps: MenuSubTriggerProps = { inset: true, disabled: false };
const menuSubContentProps: MenuSubContentProps = { placement: { side: "right", align: "start" }, offset: -4 };
const corridorHit: boolean = headingToward({ x: 1, y: 2 }, { x: 3, y: 4 }, { x: 5, y: 6, width: 7, height: 8 });

const menus = <DropdownMenu {...dropdownMenuProps}>
  <DropdownMenuTrigger {...dropdownMenuTriggerProps}>Actions</DropdownMenuTrigger>
  <DropdownMenuContent {...dropdownMenuContentProps}>
    <MenuTypeahead {...menuTypeaheadProps}>
      <MenuLabel {...menuLabelProps}>Edit</MenuLabel>
      <MenuGroup {...menuGroupProps}>
        <MenuItem {...menuItemProps}>Print</MenuItem>
        <MenuItem shortcut={null}>Plain</MenuItem>
      </MenuGroup>
      <MenuSeparator {...menuSeparatorProps} />
      <MenuCheckboxItem {...menuCheckboxItemProps}>Flag</MenuCheckboxItem>
      <MenuRadioGroup {...menuRadioGroupProps}>
        <MenuRadioItem {...menuRadioItemProps}>Y</MenuRadioItem>
      </MenuRadioGroup>
      <MenuSub {...menuSubProps}>
        <MenuSubTrigger {...menuSubTriggerProps}>Export</MenuSubTrigger>
        <MenuSubContent {...menuSubContentProps}>
          <MenuItem>Markdown</MenuItem>
        </MenuSubContent>
      </MenuSub>
      <MenuShortcut {...menuShortcutProps}>⌘P</MenuShortcut>
    </MenuTypeahead>
  </DropdownMenuContent>
  <MenuContent {...menuContentProps}><MenuItem>Orphan</MenuItem></MenuContent>
</DropdownMenu>;
type AssertSeparator<T extends true> = T;
export type MenuSeparatorHasNoChildren = AssertSeparator<Exclude<MenuSeparatorProps["children"], undefined> extends never ? true : false>;
export type MenubarSeparatorHasNoChildren = AssertSeparator<Exclude<MenubarSeparatorProps["children"], undefined> extends never ? true : false>;

const contextPoint: ContextMenuPoint = { x: 24, y: 36 };
const contextMenuProps: ContextMenuProps = { open: menuOpen(), onOpenChange: next => setMenuOpen(next) };
const contextMenuTriggerProps: ContextMenuTriggerProps = { class: "area" };
const contextMenuContentProps: ContextMenuContentProps = { placement: { side: "bottom", align: "start" }, offset: 0 };
const ContextArea = () => {
  const place: ContextMenuPlace | undefined = useContextMenuPlace();
  return <ContextMenuTrigger {...contextMenuTriggerProps} onClick={() => place?.openAt(contextPoint)}>
    Right click
  </ContextMenuTrigger>;
};
const contextMenus = <ContextMenu {...contextMenuProps}>
  <ContextArea />
  <ContextMenuContent {...contextMenuContentProps}>
    <MenuItem shortcut="Ctrl+R">Reload</MenuItem>
  </ContextMenuContent>
</ContextMenu>;

const barControl: MenubarBarControl = { step: direction => direction === 1 };
const menubarProps: MenubarProps = { label: "App bar" };
const menubarMenuProps: MenubarMenuProps = { value: "file" };
const menubarTriggerProps: MenubarTriggerProps = { class: "trigger" };
const menubarContentProps: MenubarContentProps = { placement: { side: "bottom", align: "start" }, dismissOnEscape: true };
const menubarArrowsProps: MenubarArrowsProps = { bar: barControl };
const menubarItemProps: MenubarItemProps = { closeOnSelect: false, shortcut: "Ctrl+N", onSelect: () => { void checkState; } };
const menubarCheckboxItemProps: MenubarCheckboxItemProps = { checked: checkState(), closeOnSelect: false, onCheckedChange: next => setCheckState(next) };
const menubarRadioGroupProps: MenubarRadioGroupProps = { label: "Pane", value: radioPick(), onValueChange: next => setRadioPick(next) };
const menubarRadioItemProps: MenubarRadioItemProps = { value: "split", closeOnSelect: false };
const menubarLabelProps: MenubarLabelProps = { class: "heading" };
const menubarGroupProps: MenubarGroupProps = { label: "Window" };
const menubarSeparatorProps: MenubarSeparatorProps = { class: "rule" };
const menubarShortcutProps: MenubarShortcutProps = { class: "key" };
const menubar = <Menubar {...menubarProps}>
  <MenubarMenu {...menubarMenuProps}>
    <MenubarTrigger {...menubarTriggerProps}>File</MenubarTrigger>
    <MenubarContent {...menubarContentProps}>
      <MenubarArrows {...menubarArrowsProps}>
        <MenubarLabel {...menubarLabelProps}>Commands</MenubarLabel>
        <MenubarGroup {...menubarGroupProps}>
          <MenubarItem {...menubarItemProps}>New</MenubarItem>
        </MenubarGroup>
        <MenubarSeparator {...menubarSeparatorProps} />
        <MenubarCheckboxItem {...menubarCheckboxItemProps}>Status</MenubarCheckboxItem>
        <MenubarRadioGroup {...menubarRadioGroupProps}>
          <MenubarRadioItem {...menubarRadioItemProps}>Split</MenubarRadioItem>
        </MenubarRadioGroup>
        <MenubarShortcut {...menubarShortcutProps}>⌘S</MenubarShortcut>
      </MenubarArrows>
    </MenubarContent>
  </MenubarMenu>
</Menubar>;

void gallery;
void composed;
void controls;
void themed;
void iconSizes;
void badgeVariants;
void dialogs;
void popovers;
void tooltips;
void hoverCards;
void menus;
void contextMenus;
void menubar;
