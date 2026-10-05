// NC Courier Jobs pays from its own scripts, out of TweakDB's reach. Its jobs are repeatable, so they pay like
// the other grind sources. Its own payout sliders (cj_hud.json) still apply on top.
// A separate mod, not an @if inside LynoEconomy: annotations take no module-qualified names and a conditional
// import doesn't bring the class into scope, so the import has to be plain and needs NC Courier Jobs installed.
import CourierJobs.CJ_RuntimeSystem

@wrapMethod(CJ_RuntimeSystem)
private const func JobPayoutMultiplier(jobType: String) -> Float {
  return wrappedMethod(jobType) * LynoEconomyGrindIncome();
}
