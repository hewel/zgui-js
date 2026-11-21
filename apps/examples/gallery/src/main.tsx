import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Tabs, TabsContent, TabsList, TabsTrigger, ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";
import { Masthead, SECTION_LABELS } from "./shell.js";
import type { SectionId } from "./shell.js";
import { AtomsSection } from "./sections/atoms.js";
import { FeedbackSection } from "./sections/feedback.js";
import { FieldsSection } from "./sections/fields.js";
import { ChoicesSection } from "./sections/choices.js";
import { CompositesSection } from "./sections/composites.js";
import { OverlaysSection } from "./sections/overlays.js";
import { MenusSection } from "./sections/menus.js";
import { WorkbenchSection } from "./sections/workbench.js";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <GalleryApp />, root);
}

function GalleryApp() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [preset, setPreset] = createSignal<ThemePreset>("base");
  const [section, setSection] = createSignal<SectionId>("atoms");
  const [lastAction, setLastAction] = createSignal("Ready");

  return (
    <ThemeProvider
      scheme={scheme()}
      light={{ preset: preset() }}
      dark={{ preset: preset() }}
    >
      <column testId="gallery-page" class="gallery-page">
        <Masthead
          scheme={scheme}
          setScheme={setScheme}
          preset={preset}
          setPreset={setPreset}
          lastAction={lastAction}
          onAction={setLastAction}
        />
        <Tabs
          testId="gallery-tabs"
          value={section()}
          onValueChange={value => {
            const next = value as SectionId;
            setSection(next);
            setLastAction(`Tab ${SECTION_LABELS[next]}`);
          }}
          label="Component gallery sections"
        >
          <TabsList label="Component gallery sections">
            <TabsTrigger testId="tab-atoms" value="atoms">Atoms</TabsTrigger>
            <TabsTrigger testId="tab-feedback" value="feedback">Feedback</TabsTrigger>
            <TabsTrigger testId="tab-fields" value="fields">Fields</TabsTrigger>
            <TabsTrigger testId="tab-choices" value="choices">Choices</TabsTrigger>
            <TabsTrigger testId="tab-composites" value="composites">Composites</TabsTrigger>
            <TabsTrigger testId="tab-overlays" value="overlays">Overlays</TabsTrigger>
            <TabsTrigger testId="tab-menus" value="menus">Menus</TabsTrigger>
            <TabsTrigger testId="tab-workbench" value="workbench">Workbench</TabsTrigger>
          </TabsList>
          <TabsContent value="atoms">
            <AtomsSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="feedback">
            <FeedbackSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="fields">
            <FieldsSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="choices">
            <ChoicesSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="composites">
            <CompositesSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="overlays">
            <OverlaysSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="menus">
            <MenusSection onAction={setLastAction} />
          </TabsContent>
          <TabsContent value="workbench">
            <WorkbenchSection onAction={setLastAction} />
          </TabsContent>
        </Tabs>
      </column>
    </ThemeProvider>
  );
}
