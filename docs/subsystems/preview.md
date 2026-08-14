# Preview & compute scheduling

How interactive preview stays faithful to the deterministic CPU pipeline, and how
background work is prioritized. Owning code: `focale-app` (`preview`, `jobs`,
`suggest`, `thumbs`). Governing invariants: `[HARD-DET]`
([invariants](../invariants.md)).

## One implementation of the pipeline math

Decode happens once per image. The demosaiced result is retained as a **mip
pyramid**: level 0 is the full-resolution image, and successive levels are
box-downscaled by 2 down to the **preview base** (long edge ≤ 2560 px), the
level every interactive edit renders on. Every slider change re-runs the CPU
pipeline on that base; v1 ships without per-stage caching (the seam for it is
the preview scheduler). The GPU does exactly one thing: the colour-managed
blit (working→display) plus zoom/pan sampling ([color](color.md)).

Retention is deliberate, and it is the one place preview spends memory to buy
honesty. Fit-to-window renders the preview-base level; **1:1 zoom renders the
visible tile from level 0**, because a 1:1 view whose pixels were upsampled
from a 2560 px base would misrepresent exactly the things 1:1 exists to judge
— sharpening, noise, and retouch. Budget roughly 1.33× the full-resolution
buffer for the whole pyramid (~340 MB in f32 RGB for a 60 MP frame), which is
why the pyramid is per *open* image, not per directory entry, and is dropped
when the image closes. Re-decoding on zoom instead was rejected: it puts a
full decode inside an interaction.

The stage set never varies with resolution — preview runs the export pipeline
([pipeline](pipeline.md)), so what the viewport shows is the exported file at
a different scale, not an approximation of it.

**Rationale:** the GPU preview must stay perceptually faithful to the CPU path
forever, across every pipeline version. Duplicating eleven stages in WGSL doubles
every algorithm and every version freeze. **Honesty note:** the <100 ms
slider-to-screen figure ([platform](platform.md) targets) is a *budget, not a
measurement* — no benchmark exists yet; instrumentation and a reproducible
benchmark are `v1 (gap, issue #11)`. If profiling falsifies the single-pipeline
design, the seam is the preview scheduler, not the stage code.

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
