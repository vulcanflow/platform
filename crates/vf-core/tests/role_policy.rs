//! T1a (VFL-112, covers C1 / VFL-14): the §4.2 role × action policy of
//! `vf-core::state::allowed`.
//!
//! Architecture §4.2 (via VFL-8#document-architecture). The expected table
//! below is transcribed directly from the §4.2 prose the T1a contract
//! quotes, independently of `allowed`'s own match arms:
//!
//! * `admin` — scan dispatch and cancel (any run); findings read, triage,
//!   verify; template CRUD (any); report generate and configure branding;
//!   billing full; member management.
//! * `member` — scan dispatch and cancel own; findings read, triage, verify;
//!   template CRUD own; report generate; billing read; no member
//!   management.
//!
//! Two cells are not from §4.2 directly: `Action::AuthorizationManualReview`
//! and `Action::AuthorizationRevoke` are §A3.8 admin-only authorization
//! writes, added by the architect ruling on
//! [VFL-235](/VFL/issues/VFL-235#document-decision) §3. Revocation cancels
//! every pending and active run on the target (§A3.8), and §4.2 gives
//! `member` only "cancel own", so the ruling derives `member: false` for
//! both from that rather than stating the cell itself.
//!
//! Two cells are this test's own reading of "admin has full control of the
//! tenant" (the [`Role::Admin`] doc comment) rather than a cell §4.2 spells
//! out verbatim, and are called out the way the skeleton calls out its own
//! judgment calls: an admin who may cancel *any* run or CRUD *any* template
//! can trivially do so to one that happens to be their own, so
//! `ScanCancelOwn` and `TemplateCrudOwn` are `true` for `admin` too. A ruling
//! that admin should be denied the "own" actions specifically is a dispute
//! for Cortana, not a reason to leave the cell untested.
//!
//! `viewer` is GA-absent (§4.2): [`Role::ALL`] must list only `admin` and
//! `member`, so no policy cell for it can exist to test.

use vf_core::state::{Action, Role, allowed};

#[test]
fn only_admin_and_member_are_ga_roles() {
    assert_eq!(Role::ALL.to_vec(), vec![Role::Admin, Role::Member]);
}

/// The expected value of every `(role, action)` cell, exhaustive over both
/// enums so that adding a new [`Role`] or [`Action`] variant fails this
/// test's own compilation until the new cells are decided here — the same
/// guarantee §4.2 asks of `allowed` itself.
fn expected(role: Role, action: Action) -> bool {
    use Action as A;
    use Role as R;
    match (role, action) {
        (R::Admin, A::ScanDispatch) => true,
        (R::Member, A::ScanDispatch) => true,
        (R::Admin, A::ScanCancelAny) => true,
        (R::Member, A::ScanCancelAny) => false,
        (R::Admin, A::ScanCancelOwn) => true,
        (R::Member, A::ScanCancelOwn) => true,

        (R::Admin, A::FindingRead) => true,
        (R::Member, A::FindingRead) => true,
        (R::Admin, A::FindingTriage) => true,
        (R::Member, A::FindingTriage) => true,
        (R::Admin, A::FindingVerify) => true,
        (R::Member, A::FindingVerify) => true,

        (R::Admin, A::TemplateCrudAny) => true,
        (R::Member, A::TemplateCrudAny) => false,
        (R::Admin, A::TemplateCrudOwn) => true,
        (R::Member, A::TemplateCrudOwn) => true,

        (R::Admin, A::ReportGenerate) => true,
        (R::Member, A::ReportGenerate) => true,
        (R::Admin, A::ReportConfigureBranding) => true,
        (R::Member, A::ReportConfigureBranding) => false,

        (R::Admin, A::BillingRead) => true,
        (R::Member, A::BillingRead) => true,
        (R::Admin, A::BillingManage) => true,
        (R::Member, A::BillingManage) => false,

        (R::Admin, A::MemberManage) => true,
        (R::Member, A::MemberManage) => false,

        (R::Admin, A::AuthorizationManualReview) => true,
        (R::Member, A::AuthorizationManualReview) => false,
        (R::Admin, A::AuthorizationRevoke) => true,
        (R::Member, A::AuthorizationRevoke) => false,
    }
}

#[test]
fn allowed_matches_the_section_4_2_table_for_every_cell() {
    for &role in Role::ALL {
        for &action in Action::ALL {
            assert_eq!(
                allowed(role, action),
                expected(role, action),
                "role {role:?}, action {action:?}"
            );
        }
    }
}

#[test]
fn both_roles_dispatch_scans() {
    assert!(allowed(Role::Admin, Action::ScanDispatch));
    assert!(allowed(Role::Member, Action::ScanDispatch));
}

#[test]
fn only_admin_cancels_another_members_run() {
    assert!(allowed(Role::Admin, Action::ScanCancelAny));
    assert!(!allowed(Role::Member, Action::ScanCancelAny));
}

#[test]
fn findings_and_fixes_are_identical_for_both_roles() {
    for action in [
        Action::FindingRead,
        Action::FindingTriage,
        Action::FindingVerify,
    ] {
        assert!(allowed(Role::Admin, action), "admin should have {action:?}");
        assert!(
            allowed(Role::Member, action),
            "member should have {action:?}"
        );
    }
}

#[test]
fn only_admin_crud_any_template_member_is_own_only() {
    assert!(allowed(Role::Admin, Action::TemplateCrudAny));
    assert!(!allowed(Role::Member, Action::TemplateCrudAny));
    assert!(allowed(Role::Member, Action::TemplateCrudOwn));
}

#[test]
fn only_admin_configures_report_branding() {
    assert!(allowed(Role::Admin, Action::ReportConfigureBranding));
    assert!(!allowed(Role::Member, Action::ReportConfigureBranding));
    assert!(allowed(Role::Admin, Action::ReportGenerate));
    assert!(allowed(Role::Member, Action::ReportGenerate));
}

#[test]
fn billing_is_full_for_admin_and_read_only_for_member() {
    assert!(allowed(Role::Admin, Action::BillingRead));
    assert!(allowed(Role::Admin, Action::BillingManage));
    assert!(allowed(Role::Member, Action::BillingRead));
    assert!(!allowed(Role::Member, Action::BillingManage));
}

#[test]
fn only_admin_manages_members() {
    assert!(allowed(Role::Admin, Action::MemberManage));
    assert!(!allowed(Role::Member, Action::MemberManage));
}

#[test]
fn only_admin_does_manual_review_and_authorization_revocation() {
    // Wire values quoted from the VFL-235 ruling §3, independently of
    // `Action::as_str`.
    assert_eq!(
        Action::AuthorizationManualReview.as_str(),
        "authorization_manual_review"
    );
    assert_eq!(Action::AuthorizationRevoke.as_str(), "authorization_revoke");

    assert!(allowed(Role::Admin, Action::AuthorizationManualReview));
    assert!(!allowed(Role::Member, Action::AuthorizationManualReview));
    assert!(allowed(Role::Admin, Action::AuthorizationRevoke));
    assert!(!allowed(Role::Member, Action::AuthorizationRevoke));
}
