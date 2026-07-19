# Preview & compute scheduling

How interactive preview stays faithful to the deterministic CPU pipeline, and how
background work is prioritized. Owning code: `focale-app` (`preview`, `jobs`,
`suggest`, `thumbs`). Governing invariants: `[HARD-DET]`
([invariants](../invariants.md)).

## One implementation of the pipeline math

Decode happens once per image; the result is immediately box-downscaled to a
preview base (long edge ≤ 2560 px) and the full-resolution buffer dropped. Every
slider change re-runs the CPU pipeline on that base; v1 ships without per-stage
caching (the seam for it is the preview scheduler). The GPU does exactly one
thing: the colour-managed blit (working→display) plus zoom/pan sampling
([color](color.md)).

**Rationale:** the GPU preview must stay perceptually faithful to the CPU path
forever, across every pipeline version. Duplicating eleven stages in WGSL doubles
every algorithm and every version freeze. **Honesty note:** the <100 ms
slider-to-screen figure ([platform](platform.md) targets) is a *budget, not a
measurement* — no benchmark exists yet; instrumentation and a reproducible
benchmark are `v1 (gap, issue #11)`. If profiling falsifies the single-pipeline
design, the seam is the preview scheduler, not the stage code.

Preview quality: fit-to-window renders on a mip of the demosaiced image; 1:1 zoom
renders the visible tile at full resolution.

## Job scheduler

A priority job scheduler owns all background work per opened file: interactive
preview > thumbnail/filmstrip > export queue > **idle** work.

## Suggestion engine stub

The v1 suggestion engine is a stub implementing the full v2 contract
(`v2 (committed)` for the model, [scope](../scope.md#v2-committed)): it runs when
the file's queue goes idle (or immediately on demand), produces per-slider
`Suggestion { stage, param, value }` proposals, and the UI renders
accept / tweak / ignore affordances. The stub returns "no suggestions"; the
scheduling, plumbing, and UI ship in v1. The eventual model and its training
direction live in [rnd/ml-models.md](../rnd/ml-models.md).
