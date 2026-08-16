import { lazy, Suspense } from "react";
import type { ParticleSphereProps } from "./ParticleSphere";

const ParticleSphereImpl = lazy(() => import("./ParticleSphere"));

export function ParticleSphereLazy(props: ParticleSphereProps) {
  return (
    <Suspense fallback={<div className="particle-sphere-fallback" aria-hidden />}>
      <ParticleSphereImpl {...props} />
    </Suspense>
  );
}
