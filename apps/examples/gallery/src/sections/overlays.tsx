import { createSignal } from "@hewel/zgui";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
  AlertDialogTrigger,
  Avatar,
  AvatarFallback,
  Badge,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogDismiss,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  Input,
  Label,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Tooltip,
  TooltipArrow,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@hewel/zgui/components";
import { DemoRow, Panel } from "../shell.js";

export function OverlaysSection(props: { onAction: (label: string) => void }) {
  const [multiAccordion, setMultiAccordion] = createSignal(false);
  const [collapsibleOpen, setCollapsibleOpen] = createSignal(true);
  const [projectName, setProjectName] = createSignal("zgui-gallery");
  const [widthToken, setWidthToken] = createSignal("100%");

  return (
    <row class="panel-grid" testId="section-overlays">
      <Panel
        title="Accordion & Collapsible"
        note="Keyboard-roved vertical disclosure items and standalone collapsible region"
      >
        <DemoRow label="Accordion mode">
          <Button
            size="xs"
            variant="outline"
            onClick={() => {
              const next = !multiAccordion();
              setMultiAccordion(next);
              props.onAction(`Accordion ${next ? "multiple" : "single"}`);
            }}
          >
            <text>{`Selection: ${multiAccordion() ? "multiple" : "single"}`}</text>
          </Button>
        </DemoRow>
        <Accordion
          defaultValue={["architecture"]}
          selection={multiAccordion() ? "multiple" : "single"}
          collapsible
        >
          <AccordionItem value="architecture">
            <AccordionTrigger>How does native JSX render?</AccordionTrigger>
            <AccordionContent>
              <text>
                Solid Universal talks directly to QuickJS and upstream zgui without a DOM.
              </text>
            </AccordionContent>
          </AccordionItem>
          <AccordionItem value="overlays">
            <AccordionTrigger>How are stacked layers dismissed?</AccordionTrigger>
            <AccordionContent>
              <text>
                Each surface registers a dismiss layer band so Escape closes only the top band.
              </text>
            </AccordionContent>
          </AccordionItem>
          <AccordionItem value="preview">
            <AccordionTrigger>Can I verify pixels headlessly?</AccordionTrigger>
            <AccordionContent>
              <text>
                Yes -- zgui-host preview renders real offscreen GPU frames and dumps trees.
              </text>
            </AccordionContent>
          </AccordionItem>
        </Accordion>

        <Collapsible open={collapsibleOpen()} onOpenChange={setCollapsibleOpen}>
          <CollapsibleTrigger>
            <text>{collapsibleOpen() ? "Hide build diagnostics" : "Show build diagnostics"}</text>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <column class="dashed-frame">
              <text>Entry budget: 25 ms max | Stack cap: 256 KiB | Two layout settle passes.</text>
            </column>
          </CollapsibleContent>
        </Collapsible>
      </Panel>

      <Panel
        title="Tabs Variants"
        note="Underline strip vs manual keyboard activation (Space/Enter commits tab)"
      >
        <DemoRow label="Line variant">
          <box class="demo-full">
            <Tabs defaultValue="summary" label="Report tabs">
              <TabsList variant="line" label="Report tabs">
                <TabsTrigger value="summary">Summary</TabsTrigger>
                <TabsTrigger value="metrics">Metrics</TabsTrigger>
                <TabsTrigger value="audit" disabled>
                  Audit
                </TabsTrigger>
              </TabsList>
              <TabsContent value="summary">
                <text>
                  Summary panel -- automatic activation switches immediately on Arrow keys.
                </text>
              </TabsContent>
              <TabsContent value="metrics">
                <text>Metrics panel -- underline rule highlights the active tab.</text>
              </TabsContent>
              <TabsContent value="audit">
                <text>Unavailable.</text>
              </TabsContent>
            </Tabs>
          </box>
        </DemoRow>

        <DemoRow label="Manual activation">
          <box class="demo-full">
            <Tabs defaultValue="one" activation="manual" label="Manual tabs">
              <TabsList label="Manual tabs">
                <TabsTrigger value="one">Step 1</TabsTrigger>
                <TabsTrigger value="two">Step 2</TabsTrigger>
                <TabsTrigger value="three">Step 3</TabsTrigger>
              </TabsList>
              <TabsContent value="one">
                <text>Move focus across tabs with Left/Right; press Enter or Space to open.</text>
              </TabsContent>
              <TabsContent value="two">
                <text>Step 2 activated manually.</text>
              </TabsContent>
              <TabsContent value="three">
                <text>Step 3 activated manually.</text>
              </TabsContent>
            </Tabs>
          </box>
        </DemoRow>
      </Panel>

      <Panel
        title="Dialog & Layering"
        note="Modal focus trap, nested DropdownMenu (Escape closes menu first), and custom DialogDismiss"
      >
        <DemoRow label="Modal dialogs">
          <Dialog>
            <DialogTrigger testId="overlay-dialog-trigger">Rename project...</DialogTrigger>
            <DialogContent testId="overlay-dialog-content">
              <DialogHeader>
                <DialogTitle>Rename project</DialogTitle>
                <DialogDescription>
                  Open Visibility below and press Escape once to verify layered dismissal.
                </DialogDescription>
              </DialogHeader>
              <column class="demo-stack">
                <Input
                  testId="overlay-dialog-input"
                  label="Project name"
                  value={projectName()}
                  onValueChange={setProjectName}
                />
                <DropdownMenu>
                  <DropdownMenuTrigger size="sm" variant="outline">
                    Visibility presets
                  </DropdownMenuTrigger>
                  <DropdownMenuContent class="menu-w-190">
                    <MenuLabel>Access scope</MenuLabel>
                    <MenuSeparator />
                    <MenuItem onSelect={() => props.onAction("Scope: Private")}>
                      Private workspace
                    </MenuItem>
                    <MenuItem onSelect={() => props.onAction("Scope: Team")}>Shared team</MenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </column>
              <DialogFooter>
                <DialogClose variant="outline" onClick={() => props.onAction("Dialog canceled")}>
                  Cancel
                </DialogClose>
                <DialogClose
                  variant="default"
                  onClick={() => props.onAction(`Saved: ${projectName()}`)}
                >
                  Save name
                </DialogClose>
              </DialogFooter>
            </DialogContent>
          </Dialog>

          <Dialog>
            <DialogTrigger testId="overlay-custom-dialog-trigger" variant="destructive">
              Delete token...
            </DialogTrigger>
            <DialogContent dismissControl={false} class="dialog-narrow">
              <DialogDismiss testId="overlay-custom-dismiss" />
              <DialogHeader>
                <DialogTitle>Confirm deletion</DialogTitle>
                <DialogDescription>
                  Explicit DialogDismiss renders the top-right close button when dismissControl is
                  false.
                </DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <DialogClose variant="outline">Keep</DialogClose>
                <DialogClose
                  variant="destructive"
                  onClick={() => props.onAction("Deleted demo token")}
                >
                  Delete
                </DialogClose>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </DemoRow>
        <text class="muted-caption">{`Current project name: ${projectName()}`}</text>
      </Panel>

      <Panel
        title="AlertDialog"
        note="Confirm a destructive action; clicking the backdrop keeps it open"
      >
        <DemoRow label="Destructive confirmation">
          <AlertDialog>
            <AlertDialogTrigger testId="overlay-alert-trigger" variant="destructive">
              Clear history…
            </AlertDialogTrigger>
            <AlertDialogContent testId="overlay-alert-content" size="sm">
              <AlertDialogHeader>
                <AlertDialogMedia ariaHidden>!</AlertDialogMedia>
                <AlertDialogTitle>Clear activity history?</AlertDialogTitle>
                <AlertDialogDescription>
                  This removes the demo activity history.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel testId="overlay-alert-cancel">Keep history</AlertDialogCancel>
                <AlertDialogAction
                  testId="overlay-alert-confirm"
                  variant="destructive"
                  onClick={() => props.onAction("Cleared history")}
                >
                  Clear
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </DemoRow>
      </Panel>

      <Panel
        title="Popover, Tooltip & HoverCard"
        note="Anchored floating surfaces with placement, arrow indicator, and hover handoff"
      >
        <DemoRow label="Popover">
          <Popover>
            <PopoverTrigger testId="overlay-popover-trigger" variant="outline">
              Dimensions
            </PopoverTrigger>
            <PopoverContent
              testId="overlay-popover-content"
              placement={{ side: "bottom", align: "start" }}
            >
              <PopoverHeader>
                <PopoverTitle>Surface width</PopoverTitle>
                <PopoverDescription>
                  Popper flips/shifts automatically inside the viewport.
                </PopoverDescription>
              </PopoverHeader>
              <row class="demo-pair">
                <Label>Width</Label>
                <Input
                  label="Width"
                  value={widthToken()}
                  onValueChange={setWidthToken}
                  class="input-w-130"
                />
              </row>
              <PopoverClose size="sm" onClick={() => props.onAction(`Width: ${widthToken()}`)}>
                Done
              </PopoverClose>
            </PopoverContent>
          </Popover>
          <text class="muted-caption">{`Width: ${widthToken()}`}</text>
        </DemoRow>

        <DemoRow label="TooltipProvider + Tooltip (hover or Tab focus)">
          <TooltipProvider delay={180} closeDelay={120}>
            <Tooltip>
              <TooltipTrigger>
                <Button size="sm" variant="outline" ariaLabel="Bold action">
                  Bold (Hover)
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                <text>Toggle bold weight</text>
                <TooltipArrow />
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger>
                <Button size="sm" variant="outline" ariaLabel="Inspect layers">
                  Inspect
                </Button>
              </TooltipTrigger>
              <TooltipContent>Handoff skips delay across siblings</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </DemoRow>

        <DemoRow label="HoverCard (focus or hover)">
          <HoverCard>
            <HoverCardTrigger>
              <Badge variant="secondary">@zgui-native (hover or focus)</Badge>
            </HoverCardTrigger>
            <HoverCardContent>
              <row class="demo-pair">
                <Avatar src="assets/avatar.png" label="zgui-native">
                  <AvatarFallback>ZN</AvatarFallback>
                </Avatar>
                <column class="hovercard-meta">
                  <text class="demo-strong">zgui-native</text>
                  <text class="muted-caption">GPU UI engine + QuickJS Solid host bridge</text>
                </column>
              </row>
            </HoverCardContent>
          </HoverCard>
        </DemoRow>
      </Panel>
    </row>
  );
}
