use rustc_data_structures::fx::FxHashMap;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, DefIdTree, Ty, TyCtxt};
use rustc_middle::ty::{GenericArg, GenericArgKind};
use rustc_span::Span;

use super::explicit::ExplicitPredicatesMap;
use super::utils::*;

pub(super) type GlobalInferredOutlives<'tcx> =
    FxHashMap<DefId, ty::EarlyBinder<RequiredPredicates<'tcx>>>;

/// Infer predicates for the items in the crate.
///
/// `global_inferred_outlives`: this is initially the empty map that
///     was generated by walking the items in the crate. This will
///     now be filled with inferred predicates.
pub(super) fn infer_predicates<'tcx>(tcx: TyCtxt<'tcx>) -> GlobalInferredOutlives<'tcx> {
    let mut explicit_map = ExplicitPredicatesMap::new();

    let mut global_inferred_outlives = FxHashMap::default();

    // If new predicates were added then we need to re-calculate
    // all crates since there could be new implied predicates.
    let mut round = 1;
    'outer: loop {
        debug!("infer_predicates: round {round}");
        let mut predicates_added = false;

        // Visit all the crates and infer predicates
        for id in tcx.hir().items() {
            let item_did = id.def_id;

            debug!("InferVisitor::visit_item(item={:?})", item_did);

            let mut item_required_predicates = RequiredPredicates::default();
            match tcx.def_kind(item_did) {
                DefKind::Union | DefKind::Enum | DefKind::Struct => {
                    let adt_def = tcx.adt_def(item_did.to_def_id());

                    // Iterate over all fields in item_did
                    for field_def in adt_def.all_fields() {
                        // Calculating the predicate requirements necessary
                        // for item_did.
                        //
                        // For field of type &'a T (reference) or Adt
                        // (struct/enum/union) there will be outlive
                        // requirements for adt_def.
                        let field_ty = tcx.type_of(field_def.did);
                        let field_span = tcx.def_span(field_def.did);
                        insert_required_predicates_to_be_wf(
                            tcx,
                            adt_def.did(),
                            field_ty,
                            field_span,
                            &global_inferred_outlives,
                            &mut item_required_predicates,
                            &mut explicit_map,
                        );
                    }
                }

                _ => {}
            };

            // If new predicates were added (`local_predicate_map` has more
            // predicates than the `global_inferred_outlives`), the new predicates
            // might result in implied predicates for their parent types.
            // Therefore mark `predicates_added` as true and which will ensure
            // we walk the crates again and re-calculate predicates for all
            // items.
            let item_predicates_len: usize =
                global_inferred_outlives.get(&item_did.to_def_id()).map_or(0, |p| p.0.len());
            if item_required_predicates.len() > item_predicates_len {
                predicates_added = true;
                if tracing::enabled!(tracing::Level::DEBUG) {
                    let def_id = item_did.to_def_id();
                    use std::collections::BTreeSet;
                    let global_preds: BTreeSet<_> =
                        global_inferred_outlives.get(&def_id).map_or_else(Default::default, |e| {
                            e.0.iter().map(|(pred, _)| pred).collect()
                        });
                    let computed_preds: BTreeSet<_> =
                        item_required_predicates.iter().map(|(pred, _)| pred).collect();
                    let added = computed_preds.difference(&global_preds).collect::<BTreeSet<_>>();
                    debug!("global_inferred_outlives grew for {def_id:?}, added: {added:?}");
                    let removed = global_preds.difference(&computed_preds).collect::<BTreeSet<_>>();
                    if !removed.is_empty() {
                        debug!("global_inferred_outlives lost predicates: {removed:?}")
                    }
                }
                global_inferred_outlives
                    .insert(item_did.to_def_id(), ty::EarlyBinder(item_required_predicates));
            }
        }

        if !predicates_added {
            break 'outer;
        }
        round += 1;
    }

    global_inferred_outlives
}

fn insert_required_predicates_to_be_wf<'tcx>(
    tcx: TyCtxt<'tcx>,
    self_did: DefId,
    field_ty: Ty<'tcx>,
    field_span: Span,
    global_inferred_outlives: &GlobalInferredOutlives<'tcx>,
    required_predicates: &mut RequiredPredicates<'tcx>,
    explicit_map: &mut ExplicitPredicatesMap<'tcx>,
) {
    for arg in field_ty.walk() {
        let ty = match arg.unpack() {
            GenericArgKind::Type(ty) => ty,

            // No predicates from lifetimes or constants, except potentially
            // constants' types, but `walk` will get to them as well.
            GenericArgKind::Lifetime(_) | GenericArgKind::Const(_) => continue,
        };

        match *ty.kind() {
            // The field is of type &'a T which means that we will have
            // a predicate requirement of T: 'a (T outlives 'a).
            //
            // We also want to calculate potential predicates for the T
            ty::Ref(region, rty, _) => {
                debug!("Ref");
                insert_outlives_predicate(
                    tcx,
                    rty.into(),
                    region,
                    self_did,
                    field_span,
                    required_predicates,
                    None,
                );
            }

            // For each Adt (struct/enum/union) type `Foo<'a, T>`, we
            // can load the current set of inferred and explicit
            // predicates from `global_inferred_outlives` and filter the
            // ones that are TypeOutlives.
            ty::Adt(adt, substs) => {
                // First check the inferred predicates
                //
                // Example 1:
                //
                //     struct Foo<'a, T> {
                //         field1: Bar<'a, T>
                //     }
                //
                //     struct Bar<'b, U> {
                //         field2: &'b U
                //     }
                //
                // Here, when processing the type of `field1`, we would
                // request the set of implicit predicates computed for `Bar`
                // thus far. This will initially come back empty, but in next
                // round we will get `U: 'b`. We then apply the substitution
                // `['b => 'a, U => T]` and thus get the requirement that `T:
                // 'a` holds for `Foo`.
                debug!("Adt");
                if let Some(unsubstituted_predicates) = global_inferred_outlives.get(&adt.did()) {
                    for (unsubstituted_predicate, stack) in &unsubstituted_predicates.0 {
                        // `unsubstituted_predicate` is `U: 'b` in the
                        // example above.  So apply the substitution to
                        // get `T: 'a` (or `predicate`):
                        let predicate = unsubstituted_predicates
                            .rebind(*unsubstituted_predicate)
                            .subst(tcx, substs);

                        // We must detect cycles in the inference. If we don't, rustc can hang.
                        // Cycles can be formed by associated types on traits when they are used like so:
                        //
                        // ```
                        // trait Trait<'a> { type Assoc: 'a; }
                        // struct Node<'node, T: Trait<'node>>(Var<'node, T::Assoc>, Option<T::Assoc>);
                        // struct RGen<R>(std::marker::PhantomData<R>);
                        // impl<'a, R: 'a> Trait<'a> for RGen<R> { type Assoc = R; }
                        // struct Var<'var, R: 'var>(Box<Node<'var, RGen<R>>>);
                        // ```
                        //
                        // Visiting Node, we walk the fields and find a Var. Var has an explicit
                        //     R                         : 'var.
                        // Node finds this on its Var field, substitutes through, and gets an inferred
                        //     <T as Trait<'node>>::Assoc: 'node.
                        // Visiting Var, we walk the fields and find a Node. So Var then picks up
                        // Node's new inferred predicate (in global_inferred_outlives) and substitutes
                        // the types it passed to Node ('var for 'node, RGen<R> for T).
                        // So Var gets
                        //     <RGen<R> as Trait<'var>>::Assoc: 'var
                        // But Node contains a Var. So Node gets
                        //     <RGen<<T as Trait<'node>>::Assoc> as Trait<'node>>::Assoc 'node
                        // Var gets
                        //     <RGen<<RGen<R> as Trait<'var>>::Assoc> as Trait<'var>>::Assoc: 'var
                        // Etc. This goes on forever.
                        //
                        // We cut off the cycle formation by tracking in a stack the defs that
                        // have picked up a substituted predicate each time we produce an edge,
                        // and don't insert a predicate that is simply a substituted version of
                        // one we've already seen and added.
                        //
                        // Try: RUSTC_LOG=rustc_hir_analysis::outlives=debug \
                        //      rustc +stage1 src/test/ui/typeck/issue-102966.rs 2>&1 \
                        //      | rg '(grew|cycle)'
                        //
                        // We do not currently treat a type with an explicit bound as the first
                        // in the visit stack. So Var here does not appear first in the stack,
                        // Node does, and each of Node and Var will get a version of
                        // `<RGen<R> as Trait<'node>>::Assoc: 'node` before the cycle is cut at
                        // Node. This avoids having a second equivalent bound on Node, and also
                        // having RGen involved in Node's predicates (which would be silly).
                        //
                        // It is not clear whether cyclically-substituted versions of bounds we
                        // already have are always redundant/unnecessary to add to Self.
                        // This solution avoids digging into `impl Trait for RGen` to find that it
                        // unifies with an existing bound but it is really a guess that this
                        // cyclic substitution cannot add valuable information. There may be
                        // situations when an error is appropriate.
                        if stack.iter().any(|&(did, _span)| did == self_did) {
                            debug!(
                                "detected cycle in inferred_outlives_predicates,\
                                for unsubstituted predicate {unsubstituted_predicate:?}:\
                                {self_did:?} found in {stack:?}"
                            );
                        } else {
                            insert_outlives_predicate(
                                tcx,
                                predicate.0,
                                predicate.1,
                                // Treat the top-level definition we are currently walking the fields of as the
                                // type visited in the DefStack. Not the field type.
                                self_did,
                                field_span,
                                required_predicates,
                                // a copy of this is made for the predicate and (self_did, field_span) is pushed.
                                Some(stack),
                            );
                        }
                    }
                }

                // Check if the type has any explicit predicates that need
                // to be added to `required_predicates`
                // let _: () = substs.region_at(0);
                check_explicit_predicates(
                    tcx,
                    self_did,
                    adt.did(),
                    substs,
                    required_predicates,
                    explicit_map,
                    None,
                );
            }

            ty::Dynamic(obj, ..) => {
                // This corresponds to `dyn Trait<..>`. In this case, we should
                // use the explicit predicates as well.

                debug!("Dynamic");
                debug!("field_ty = {}", &field_ty);
                debug!("ty in field = {}", &ty);
                if let Some(ex_trait_ref) = obj.principal() {
                    // Here, we are passing the type `usize` as a
                    // placeholder value with the function
                    // `with_self_ty`, since there is no concrete type
                    // `Self` for a `dyn Trait` at this
                    // stage. Therefore when checking explicit
                    // predicates in `check_explicit_predicates` we
                    // need to ignore checking the explicit_map for
                    // Self type.
                    let substs =
                        ex_trait_ref.with_self_ty(tcx, tcx.types.usize).skip_binder().substs;
                    check_explicit_predicates(
                        tcx,
                        self_did,
                        ex_trait_ref.skip_binder().def_id,
                        substs,
                        required_predicates,
                        explicit_map,
                        Some(tcx.types.self_param),
                    );
                }
            }

            ty::Projection(obj) => {
                // This corresponds to `<T as Foo<'a>>::Bar`. In this case, we should use the
                // explicit predicates as well.
                debug!("Projection");
                check_explicit_predicates(
                    tcx,
                    self_did,
                    tcx.parent(obj.item_def_id),
                    obj.substs,
                    required_predicates,
                    explicit_map,
                    None,
                );
            }

            _ => {}
        }
    }
}

/// We also have to check the explicit predicates
/// declared on the type.
/// ```ignore (illustrative)
/// struct Foo<'a, T> {
///     field1: Bar<T>
/// }
///
/// struct Bar<U> where U: 'static, U: Foo {
///     ...
/// }
/// ```
/// Here, we should fetch the explicit predicates, which
/// will give us `U: 'static` and `U: Foo`. The latter we
/// can ignore, but we will want to process `U: 'static`,
/// applying the substitution as above.
fn check_explicit_predicates<'tcx>(
    tcx: TyCtxt<'tcx>,
    // i.e. Foo
    self_did: DefId,
    // i.e. Bar
    def_id: DefId,
    substs: &[GenericArg<'tcx>],
    required_predicates: &mut RequiredPredicates<'tcx>,
    explicit_map: &mut ExplicitPredicatesMap<'tcx>,
    ignored_self_ty: Option<Ty<'tcx>>,
) {
    debug!(
        "check_explicit_predicates(\
         self_did={:?},\
         def_id={:?}, \
         substs={:?}, \
         explicit_map={:?}, \
         required_predicates={:?}, \
         ignored_self_ty={:?})",
        self_did, def_id, substs, explicit_map, required_predicates, ignored_self_ty,
    );
    let explicit_predicates = explicit_map.explicit_predicates_of(tcx, self_did, def_id);

    for (outlives_predicate, stack) in &explicit_predicates.0 {
        debug!("outlives_predicate = {:?}", &outlives_predicate);

        // Careful: If we are inferring the effects of a `dyn Trait<..>`
        // type, then when we look up the predicates for `Trait`,
        // we may find some that reference `Self`. e.g., perhaps the
        // definition of `Trait` was:
        //
        // ```
        // trait Trait<'a, T> where Self: 'a  { .. }
        // ```
        //
        // we want to ignore such predicates here, because
        // there is no type parameter for them to affect. Consider
        // a struct containing `dyn Trait`:
        //
        // ```
        // struct MyStruct<'x, X> { field: Box<dyn Trait<'x, X>> }
        // ```
        //
        // The `where Self: 'a` predicate refers to the *existential, hidden type*
        // that is represented by the `dyn Trait`, not to the `X` type parameter
        // (or any other generic parameter) declared on `MyStruct`.
        //
        // Note that we do this check for self **before** applying `substs`. In the
        // case that `substs` come from a `dyn Trait` type, our caller will have
        // included `Self = usize` as the value for `Self`. If we were
        // to apply the substs, and not filter this predicate, we might then falsely
        // conclude that e.g., `X: 'x` was a reasonable inferred requirement.
        //
        // Another similar case is where we have an inferred
        // requirement like `<Self as Trait>::Foo: 'b`. We presently
        // ignore such requirements as well (cc #54467)-- though
        // conceivably it might be better if we could extract the `Foo
        // = X` binding from the object type (there must be such a
        // binding) and thus infer an outlives requirement that `X:
        // 'b`.
        if let Some(self_ty) = ignored_self_ty
            && let GenericArgKind::Type(ty) = outlives_predicate.0.unpack()
            && ty.walk().any(|arg| arg == self_ty.into())
        {
            debug!("skipping self ty = {:?}", &ty);
            continue;
        }

        let &(_foo_did, span) = stack.last().unwrap();
        let predicate = explicit_predicates.rebind(*outlives_predicate).subst(tcx, substs);
        debug!("predicate = {:?}", &predicate);
        insert_outlives_predicate(
            tcx,
            predicate.0,
            predicate.1,
            // i.e. Foo, not the field ADT definition.
            self_did,
            span,
            required_predicates,
            None,
        );
    }
}
