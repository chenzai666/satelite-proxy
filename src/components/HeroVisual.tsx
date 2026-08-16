import { useTheme } from "../theme";
import type { ParticleSphereState } from "./ParticleSphere";
import { ParticleSphereLazy } from "./ParticleSphereLazy";

interface Props {
  state: ParticleSphereState;
  spinning: boolean;
  switching: boolean;
  variant?: "pro" | "simple";
}

export function HeroVisual({
  state,
  spinning,
  switching,
  variant = "pro",
}: Props) {
  const { heroStyle } = useTheme();
  const simple = variant === "simple";

  if (heroStyle === "particle") {
    return (
      <div
        className={`orbit particle-orbit ${simple ? "simple-orbit" : ""} ${spinning ? "spin" : ""} ${switching ? "pulse switching" : ""}`}
        aria-hidden
      >
        <ParticleSphereLazy
          state={state}
          count={simple ? 1800 : 2400}
          size={simple ? 1.84 : 1.96}
        />
      </div>
    );
  }

  return (
    <div
      className={`orbit ${simple ? "simple-orbit" : ""} ${spinning ? "spin" : ""} ${switching ? "pulse switching" : ""}`}
      aria-hidden
    >
      <div className="orbit-ring orbit-ring-a" />
      <div className="orbit-ring orbit-ring-b" />
      <div className="orbit-core">
        {switching ? (
          <span className="lat-spinner orbit-core-spinner" aria-hidden />
        ) : (
          <span className={`orbit-glyph ${simple ? "simple-orbit-power" : ""}`}>
            {simple ? "⏻" : "◈"}
          </span>
        )}
      </div>
      <div className="orbit-sat" />
    </div>
  );
}
