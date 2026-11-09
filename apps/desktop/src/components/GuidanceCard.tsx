import type { Guidance } from '../lib/types';
export function GuidanceCard({ guidance }: { guidance: Guidance }) {
  return (
    <section className="guidance-card" aria-label="处理建议">
      <strong>{guidance.title}</strong>
      <p>{guidance.explanation}</p>
      <ol>
        {guidance.steps.map((step) => (
          <li key={step}>{step}</li>
        ))}
      </ol>
    </section>
  );
}
