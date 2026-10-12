<script lang="ts">
  let {
    label,
    value,
    step,
    unit,
    onchange,
  }: { label: string; value: number; step: number; unit: string; onchange: (v: number) => void } = $props();
</script>

<div class="stepper">
  <span class="label">{label}</span>
  <button class="step" aria-label="Decrease {label}" onclick={() => onchange(value - step)}>−</button>
  <button class="value" title="Reset" onclick={() => onchange(0)}>{value} {unit}</button>
  <button class="step" aria-label="Increase {label}" onclick={() => onchange(value + step)}>+</button>
</div>

<style>
  .stepper {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 10px;
  }
  .label {
    flex: 1;
    color: var(--muted);
    font-size: 12px;
  }
  .step {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    background: var(--hover);
    color: var(--text);
  }
  .step:hover {
    background: var(--hover-strong);
  }
  .value {
    min-width: 72px;
    font-variant-numeric: tabular-nums;
    color: var(--text);
    font-size: 12px;
  }
</style>
