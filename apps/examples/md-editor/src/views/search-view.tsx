import { For, Show } from "@hewel/zgui";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Input,
} from "@hewel/zgui/components";
import { useNavigate } from "@hewel/zgui/router";
import { useStudio } from "../studio-context.js";

export function SearchView() {
  const studio = useStudio();
  const navigate = useNavigate();

  return (
    <scroll class="mde-page-scroll" testId="view-search">
      <column class="mde-page-inner">
        <row style={{ "justify-content": "space-between", "align-items": "center" }}>
          <column style={{ gap: "4px" }}>
            <text style={{ "font-size": "20px", "font-weight": "700" }}>
              Workspace Full-Text Search
            </text>
            <text class="muted">
              Powered by app-scoped SQLite FTS5 across local workspace files and built-in notes.
            </text>
          </column>
          <Badge testId="fts-indexed-count" variant="secondary">
            {`Indexed: ${studio.indexedCount()} docs`}
          </Badge>
        </row>

        <row style={{ gap: "8px", "align-items": "center" }}>
          <box style={{ flex: 1 }}>
            <Input
              testId="fts-search-input"
              value={studio.searchQuery()}
              onValueChange={q => void studio.runSearch(q)}
              placeholder="Search notes (e.g. Wayland, SQLite, 渲染管线)..."
              label="Full-text search query"
            />
          </box>
          <Button
            testId="fts-search-btn"
            onClick={() => void studio.runSearch(studio.searchQuery())}
          >
            Search
          </Button>
          <Button
            testId="fts-clear-btn"
            variant="outline"
            onClick={() => void studio.runSearch("")}
          >
            Clear
          </Button>
        </row>

        <text class="muted" testId="fts-hit-count">
          {studio.searchQuery().trim()
            ? `${studio.searchHits().length} matching document(s) for "${studio.searchQuery().trim()}"`
            : "Type a search term above to query SQLite FTS5."}
        </text>

        <Show when={studio.searchHits().length > 0}>
          <column style={{ gap: "10px" }}>
            <For each={studio.searchHits()}>
              {(hit, idx) => (
                <Card testId={`fts-hit-${idx()}`}>
                  <CardHeader>
                    <row
                      style={{
                        "justify-content": "space-between",
                        "align-items": "center",
                        width: "100%",
                      }}
                    >
                      <CardTitle>{hit.title}</CardTitle>
                      <row style={{ gap: "8px", "align-items": "center" }}>
                        <Badge variant="outline">{hit.path}</Badge>
                        <Button
                          testId={`fts-open-${idx()}`}
                          size="xs"
                          onClick={() => {
                            studio.setActiveDocId(hit.docId);
                            studio.setStatusMsg(`Opened search hit "${hit.title}"`);
                            void navigate("/editor");
                          }}
                        >
                          Open
                        </Button>
                      </row>
                    </row>
                  </CardHeader>
                  <CardContent>
                    <text class="mono">{hit.excerpt}</text>
                  </CardContent>
                </Card>
              )}
            </For>
          </column>
        </Show>
      </column>
    </scroll>
  );
}
