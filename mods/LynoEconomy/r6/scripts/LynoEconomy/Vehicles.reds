// One price per car at every dealer. The game's Autofixer reads EconomicAssignment.<vehicle>.overrideValue,
// modded cars carry Vehicle.<vehicle>.dealerPrice (SDH0 cars, Autofixer Customs), and Garage, Chop Shop and NCGT
// derive their buy, claim, sell and service prices from those two flats. Scaling them here moves all of them at once.
// Dedka keeps its own catalog (cars/ra1.json) and its own prices.
func LynoEconomyVehicleMultiplier() -> Float = 1.5;

public class LynoEconomyVehicles extends ScriptableTweak {
  // Several offers and vehicles can point at one EconomicAssignment: scale it once.
  private let done: array<TweakDBID>;

  protected cb func OnApply() -> Void {
    let assignments = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataVehicleOffer_Record") {
      let price = (record as VehicleOffer_Record).Price();
      if IsDefined(price) {
        assignments += this.ScaleAssignment(price.GetID());
      }
    }
    let dealer = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataVehicle_Record") {
      // Garage and Chop Shop look the assignment up by name even for cars no dealer sells (quest cars they let you claim).
      assignments += this.ScaleAssignment(LynoEconomyAssignmentOf(record.GetID()));
      dealer += LynoEconomyScaleIntFlat(record.GetID() + t".dealerPrice", LynoEconomyVehicleMultiplier());
    }
    LogChannel(n"DEBUG", s"[LynoEconomy] vehicles: \(assignments) Autofixer prices, \(dealer) dealer prices scaled");
  }

  private func ScaleAssignment(id: TweakDBID) -> Int32 {
    if ArrayContains(this.done, id) {
      return 0;
    }
    ArrayPush(this.done, id);
    let scaled = LynoEconomyScaleIntFlat(id + t".overrideValue", LynoEconomyVehicleMultiplier());
    if scaled > 0 {
      TweakDBManager.UpdateRecord(id);
    }
    return scaled;
  }
}

// Virtual Car Dealer writes its buy-back prices (Vehicle.<vehicle>.autofixer, copies of the game's prices) from its own
// ScriptableTweak, and the order of scriptable tweaks isn't defined. So they are synced once a game is running:
// set to the already scaled Autofixer price, which is idempotent across loads. NCGT reads the same flat for service costs.
public class LynoEconomyVehicleSync extends ScriptableSystem {
  private func OnAttach() -> Void {
    let synced = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataVehicle_Record") {
      let buyBack = TweakDBInterface.GetInt(record.GetID() + t".autofixer", 0);
      let price = TweakDBInterface.GetInt(LynoEconomyAssignmentOf(record.GetID()) + t".overrideValue", 0);
      if buyBack > 0 && price > 0 && buyBack != price {
        TweakDBManager.SetFlat(record.GetID() + t".autofixer", price);
        synced += 1;
      }
    }
    LogChannel(n"DEBUG", s"[LynoEconomy] vehicles: \(synced) Virtual Car Dealer prices synced");
  }
}

// Vehicle.v_sport1_herrera_outlaw_player -> EconomicAssignment.v_sport1_herrera_outlaw_player
func LynoEconomyAssignmentOf(vehicle: TweakDBID) -> TweakDBID {
  return TDBID.Create("EconomicAssignment." + StrAfterFirst(TDBID.ToStringDEBUG(vehicle), "."));
}

func LynoEconomyScaleIntFlat(flat: TweakDBID, k: Float) -> Int32 {
  let v = TweakDBInterface.GetFlat(flat);
  if !IsDefined(v) || !Equals(VariantTypeName(v), n"Int32") || FromVariant<Int32>(v) <= 0 {
    return 0;
  }
  return TweakDBManager.SetFlat(flat, RoundF(Cast<Float>(FromVariant<Int32>(v)) * k)) ? 1 : 0;
}
