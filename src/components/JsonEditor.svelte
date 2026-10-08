<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { basicSetup } from "codemirror";
  import { EditorState, Prec } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { json } from "@codemirror/lang-json";
  import { highlight, keymap, theme } from "../lib/codemirror";

  let {
    value,
    onchange,
    onsubmit,
    readonly = false,
  }: { value: string; onchange?: (v: string) => void; onsubmit?: () => void; readonly?: boolean } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | undefined;

  onMount(() => {
    view = new EditorView({
      parent: host!,
      state: EditorState.create({
        doc: value,
        extensions: [
          Prec.highest(keymap.of([{ key: "Mod-Enter", run: () => (onsubmit?.(), true) }])),
          basicSetup,
          json(),
          theme,
          highlight,
          EditorState.readOnly.of(readonly),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onchange?.(u.state.doc.toString());
          }),
        ],
      }),
    });
  });

  // Replace the text when the parent loads a different value.
  $effect(() => {
    const v = value;
    if (view && v !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: v } });
    }
  });

  onDestroy(() => view?.destroy());

  export function focus() {
    view?.focus();
  }
</script>

<div class="je" bind:this={host}></div>

<style>
  .je {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .je :global(.cm-editor) {
    height: 100%;
  }
</style>
