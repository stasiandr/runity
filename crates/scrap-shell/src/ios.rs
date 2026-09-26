//! The shell on iOS: the scene lifecycle UIKit asks for.
//!
//! Since iOS 27 UIKit stops an app linked against its SDK that has not
//! adopted scenes — `UIApplicationSceneManifest` in its Info.plist and a
//! scene delegate. winit (0.30 and 0.31) still makes its `UIWindow` the
//! old way, with no scene. This is the least that bridges the two: a scene
//! delegate, `ScrapSceneDelegate`, that puts winit's window into the scene
//! when the scene connects (or the window into the scene already there,
//! when the window comes second), and makes it key and visible.
//!
//! The app's Info.plist names it:
//!
//! ```text
//! UIApplicationSceneManifest
//!   UIApplicationSupportsMultipleScenes  false
//!   UISceneConfigurations
//!     UIWindowSceneSessionRoleApplication
//!       - UISceneConfigurationName   Default
//!         UISceneDelegateClassName   ScrapSceneDelegate
//! ```
//!
//! The longer road (docs/stack.md, rule 1) is a `UIViewController` of the
//! engine's own that owns the view; winit is the convenience path here as
//! on the desktop.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{define_class, ClassType, MainThreadOnly, Message};
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{
    UIResponder, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIView,
    UIWindow, UIWindowScene, UIWindowSceneDelegate,
};

thread_local! {
    /// winit's window, once made, and the scene, once connected: whichever
    /// comes second joins them.
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
    static SCENE: RefCell<Option<Retained<UIWindowScene>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(UIResponder, NSObject))]
    #[name = "ScrapSceneDelegate"]
    #[thread_kind = MainThreadOnly]
    struct SceneDelegate;

    unsafe impl NSObjectProtocol for SceneDelegate {}

    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn will_connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
            let Ok(scene) = scene.retain().downcast::<UIWindowScene>() else {
                return;
            };
            SCENE.with(|s| *s.borrow_mut() = Some(scene));
            join();
        }
    }

    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

/// Register the delegate's class with the runtime, before UIKit looks for
/// it by the name the Info.plist gives.
pub(crate) fn register() {
    let _ = SceneDelegate::class();
}

/// winit made its window: `view` is the window's `UIView` (from its
/// raw window handle).
pub(crate) fn window_made(view: *mut std::ffi::c_void) {
    // SAFETY: winit's UiKit handle is its window's view, alive with it.
    let view: &UIView = unsafe { &*(view as *const UIView) };
    if let Some(window) = view.window() {
        WINDOW.with(|w| *w.borrow_mut() = Some(window));
        join();
    }
}

fn join() {
    let window = WINDOW.with(|w| w.borrow().clone());
    let scene = SCENE.with(|s| s.borrow().clone());
    if let (Some(window), Some(scene)) = (window, scene) {
        window.setWindowScene(Some(&scene));
        window.makeKeyAndVisible();
    }
}
