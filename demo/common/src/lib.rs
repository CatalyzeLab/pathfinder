// pathfinder/demo/common/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A demo app for Pathfinder.

#[macro_use]
extern crate log;

use crate::camera::{Camera, Mode};
use crate::concurrent::DemoExecutor;
use crate::device::{GroundProgram, GroundVertexArray};
use crate::ui::{DemoUI, UIAction};
use crate::window::{Event, Keycode, SVGPath, View, Window, WindowSize};
use clap::{App, Arg};
use image::ColorType;
use pathfinder_geometry::basic::point::{Point2DF32, Point2DI32};
use pathfinder_geometry::basic::rect::RectF32;
use pathfinder_geometry::basic::transform3d::Transform3DF32;
use pathfinder_geometry::color::{ColorF, ColorU};
use pathfinder_gl::GLDevice;
use pathfinder_gpu::resources::ResourceLoader;
use pathfinder_gpu::{ClearParams, DepthFunc, DepthState, Device, Primitive, RenderState};
use pathfinder_gpu::{TextureFormat, UniformData};
use pathfinder_renderer::concurrent::scene_proxy::{RenderCommandStream, SceneProxy};
use pathfinder_renderer::gpu::renderer::{DestFramebuffer, RenderMode, RenderStats, Renderer};
use pathfinder_renderer::gpu_data::RenderCommand;
use pathfinder_renderer::options::{RenderOptions, RenderTransform};
use pathfinder_renderer::post::{DEFRINGING_KERNEL_CORE_GRAPHICS, STEM_DARKENING_FACTORS};
use pathfinder_renderer::scene::Scene;
use pathfinder_svg::BuiltSVG;
use pathfinder_ui::{MousePosition, UIEvent};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use usvg::{Options as UsvgOptions, Tree};

static DEFAULT_SVG_VIRTUAL_PATH: &'static str = "svg/Ghostscript_Tiger.svg";

const MOUSELOOK_ROTATION_SPEED: f32 = 0.007;
const CAMERA_VELOCITY: f32 = 0.02;

// How much the scene is scaled when a scale gesture is performed.
const CAMERA_SCALE_SPEED_2D: f32 = 6.0;
// How much the scene is scaled when a zoom button is clicked.
const CAMERA_ZOOM_AMOUNT_2D: f32 = 0.1;

const LIGHT_BG_COLOR: ColorU = ColorU {
    r: 248,
    g: 248,
    b: 248,
    a: 255,
};
const DARK_BG_COLOR: ColorU = ColorU {
    r: 32,
    g: 32,
    b: 32,
    a: 255,
};
const TRANSPARENT_BG_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

const GROUND_SOLID_COLOR: ColorU = ColorU {
    r: 80,
    g: 80,
    b: 80,
    a: 255,
};
const GROUND_LINE_COLOR: ColorU = ColorU {
    r: 127,
    g: 127,
    b: 127,
    a: 255,
};

const APPROX_FONT_SIZE: f32 = 16.0;

const MESSAGE_TIMEOUT_SECS: u64 = 5;

pub const GRIDLINE_COUNT: i32 = 10;

pub mod window;

mod camera;
mod concurrent;
mod device;
mod ui;

pub struct DemoApp<W> where W: Window {
    pub window: W,
    pub should_exit: bool,
    pub options: Options,

    window_size: WindowSize,

    scene_metadata: SceneMetadata,
    render_transform: Option<RenderTransform>,
    render_command_stream: Option<RenderCommandStream>,

    camera: Camera,
    frame_counter: u32,
    pending_screenshot_path: Option<PathBuf>,
    mouselook_enabled: bool,
    pub dirty: bool,
    expire_message_event_id: u32,
    message_epoch: u32,
    last_mouse_position: Point2DI32,

    current_frame: Option<Frame>,
    build_time: Option<Duration>,

    ui: DemoUI<GLDevice>,
    scene_proxy: SceneProxy,
    renderer: Renderer<GLDevice>,

    scene_framebuffer: Option<<GLDevice as Device>::Framebuffer>,

    ground_program: GroundProgram<GLDevice>,
    ground_vertex_array: GroundVertexArray<GLDevice>,
}

impl<W> DemoApp<W> where W: Window {
    pub fn new(window: W, window_size: WindowSize, mut options: Options) -> DemoApp<W> {
        let expire_message_event_id = window.create_user_event_id();

        let device = GLDevice::new(window.gl_version(), window.gl_default_framebuffer());
        let resources = window.resource_loader();

        // Read command line options.
        options.command_line_overrides();

        // Set up the executor.
        let executor = DemoExecutor::new(options.jobs);

        let mut built_svg = load_scene(resources, &options.input_path);
        let message = get_svg_building_message(&built_svg);

        let viewport = window.viewport(options.mode.view(0));
        let dest_framebuffer = DestFramebuffer::Default {
            viewport,
            window_size: window_size.device_size(),
        };

        let renderer = Renderer::new(device, resources, dest_framebuffer);
        let scene_metadata = SceneMetadata::new_clipping_view_box(&mut built_svg.scene,
                                                                  viewport.size());
        let camera = Camera::new(options.mode, scene_metadata.view_box, viewport.size());

        let scene_proxy = SceneProxy::new(built_svg.scene, executor);

        let ground_program = GroundProgram::new(&renderer.device, resources);
        let ground_vertex_array = GroundVertexArray::new(&renderer.device,
                                                         &ground_program,
                                                         &renderer.quad_vertex_positions_buffer());

        let mut ui = DemoUI::new(&renderer.device, resources, options.clone());
        let mut message_epoch = 0;
        emit_message::<W>(
            &mut ui,
            &mut message_epoch,
            expire_message_event_id,
            message,
        );

        DemoApp {
            window,
            should_exit: false,
            options,

            window_size,

            scene_metadata,
            render_transform: None,
            render_command_stream: None,

            camera,
            frame_counter: 0,
            pending_screenshot_path: None,
            mouselook_enabled: false,
            dirty: true,
            expire_message_event_id,
            message_epoch,
            last_mouse_position: Point2DI32::default(),

            current_frame: None,
            build_time: None,

            ui,
            scene_proxy,
            renderer,

            scene_framebuffer: None,

            ground_program,
            ground_vertex_array,
        }
    }

    pub fn prepare_frame(&mut self, events: Vec<Event>) -> u32 {
        // Clear dirty flag.
        self.dirty = false;

        // Handle events.
        let ui_events = self.handle_events(events);

        // Update the scene.
        self.build_scene();

        // Save the frame.
        //
        // FIXME(pcwalton): This is super ugly.
        let transform = self.render_transform.clone().unwrap();
        self.current_frame = Some(Frame::new(transform, ui_events));

        // Initialize and set the appropriate framebuffer.
        let view = self.ui.mode.view(0);
        self.window.make_current(view);
        let window_size = self.window_size.device_size();
        let scene_count = match self.camera.mode() {
            Mode::VR => {
                let viewport = self.window.viewport(View::Stereo(0));
                if self.scene_framebuffer.is_none()
                    || self.renderer.device.texture_size(
                        &self
                            .renderer
                            .device
                            .framebuffer_texture(self.scene_framebuffer.as_ref().unwrap()),
                    ) != viewport.size()
                {
                    let scene_texture = self
                        .renderer
                        .device
                        .create_texture(TextureFormat::RGBA8, viewport.size());
                    self.scene_framebuffer =
                        Some(self.renderer.device.create_framebuffer(scene_texture));
                }
                self.renderer
                    .replace_dest_framebuffer(DestFramebuffer::Other(
                        self.scene_framebuffer.take().unwrap(),
                    ));
                2
            }
            _ => {
                self.renderer
                    .replace_dest_framebuffer(DestFramebuffer::Default {
                        viewport: self.window.viewport(View::Mono),
                        window_size,
                    });
                1
            }
        };

        // Begin drawing the scene.
        self.renderer.bind_dest_framebuffer();

        // Clear to the appropriate color.
        let clear_color = if scene_count == 2 {
            ColorF::transparent_black()
        } else {
            self.background_color().to_f32()
        };
        self.renderer.device.clear(&ClearParams {
            color: Some(clear_color),
            depth: Some(1.0),
            stencil: Some(0),
            ..ClearParams::default()
        });

        scene_count
    }

    fn build_scene(&mut self) {
        self.render_transform = match self.camera {
            Camera::ThreeD {
                ref scene_transform,
                ref mut modelview_transform,
                ref mut velocity,
                ..
            } => {
                if modelview_transform.offset(*velocity) {
                    self.dirty = true;
                }
                let perspective = scene_transform
                    .perspective
                    .post_mul(&scene_transform.modelview_to_eye)
                    .post_mul(&modelview_transform.to_transform());
                Some(RenderTransform::Perspective(perspective))
            }
            Camera::TwoD(transform) => Some(RenderTransform::Transform2D(transform)),
        };

        let render_options = RenderOptions {
            transform: self.render_transform.clone().unwrap(),
            dilation: if self.ui.stem_darkening_effect_enabled {
                let font_size = APPROX_FONT_SIZE * self.window_size.backing_scale_factor;
                let (x, y) = (STEM_DARKENING_FACTORS[0], STEM_DARKENING_FACTORS[1]);
                Point2DF32::new(x, y).scale(font_size)
            } else {
                Point2DF32::default()
            },
            subpixel_aa_enabled: self.ui.subpixel_aa_effect_enabled,
        };

        let built_options = render_options.prepare(self.scene_metadata.bounds);
        self.render_command_stream = Some(self.scene_proxy.build_with_stream(built_options));
    }

    fn handle_events(&mut self, events: Vec<Event>) -> Vec<UIEvent> {
        let mut ui_events = vec![];
        self.dirty = false;

        for event in events {
            match event {
                Event::Quit { .. } | Event::KeyDown(Keycode::Escape) => {
                    self.should_exit = true;
                    self.dirty = true;
                }
                Event::WindowResized(new_size) => {
                    self.window_size = new_size;
                    let viewport = self.window.viewport(self.ui.mode.view(0));
                    self.scene_proxy.set_view_box(RectF32::new(Point2DF32::default(),
                                                               viewport.size().to_f32()));
                    self.renderer
                        .set_main_framebuffer_size(self.window_size.device_size());
                    self.dirty = true;
                }
                Event::MouseDown(new_position) => {
                    let mouse_position = self.process_mouse_position(new_position);
                    ui_events.push(UIEvent::MouseDown(mouse_position));
                }
                Event::MouseMoved(new_position) if self.mouselook_enabled => {
                    let mouse_position = self.process_mouse_position(new_position);
                    if let Camera::ThreeD {
                        ref mut modelview_transform,
                        ..
                    } = self.camera
                    {
                        let rotation = mouse_position
                            .relative
                            .to_f32()
                            .scale(MOUSELOOK_ROTATION_SPEED);
                        modelview_transform.yaw += rotation.x();
                        modelview_transform.pitch += rotation.y();
                        self.dirty = true;
                    }
                }
                Event::MouseDragged(new_position) => {
                    let mouse_position = self.process_mouse_position(new_position);
                    ui_events.push(UIEvent::MouseDragged(mouse_position));
                    self.dirty = true;
                }
                Event::Zoom(d_dist, position) => {
                    if let Camera::TwoD(ref mut transform) = self.camera {
                        let backing_scale_factor = self.window_size.backing_scale_factor;
                        let position = position.to_f32().scale(backing_scale_factor);
                        *transform = transform.post_translate(-position);
                        let scale_delta = 1.0 + d_dist * CAMERA_SCALE_SPEED_2D;
                        *transform = transform.post_scale(Point2DF32::splat(scale_delta));
                        *transform = transform.post_translate(position);
                    }
                }
                Event::Look { pitch, yaw } => {
                    if let Camera::ThreeD {
                        ref mut modelview_transform,
                        ..
                    } = self.camera
                    {
                        modelview_transform.pitch += pitch;
                        modelview_transform.yaw += yaw;
                    }
                }
                Event::SetEyeTransforms(new_eye_transforms) => {
                    if let Camera::ThreeD {
                        ref mut eye_transforms,
                        ..
                    } = self.camera
                    {
                        *eye_transforms = new_eye_transforms;
                    }
                }
                Event::KeyDown(Keycode::Alphanumeric(b'w')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        let scale_factor =
                            camera::scale_factor_for_view_box(self.scene_metadata.view_box);
                        velocity.set_z(-CAMERA_VELOCITY / scale_factor);
                        self.dirty = true;
                    }
                }
                Event::KeyDown(Keycode::Alphanumeric(b's')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        let scale_factor =
                            camera::scale_factor_for_view_box(self.scene_metadata.view_box);
                        velocity.set_z(CAMERA_VELOCITY / scale_factor);
                        self.dirty = true;
                    }
                }
                Event::KeyDown(Keycode::Alphanumeric(b'a')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        let scale_factor =
                            camera::scale_factor_for_view_box(self.scene_metadata.view_box);
                        velocity.set_x(-CAMERA_VELOCITY / scale_factor);
                        self.dirty = true;
                    }
                }
                Event::KeyDown(Keycode::Alphanumeric(b'd')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        let scale_factor =
                            camera::scale_factor_for_view_box(self.scene_metadata.view_box);
                        velocity.set_x(CAMERA_VELOCITY / scale_factor);
                        self.dirty = true;
                    }
                }
                Event::KeyUp(Keycode::Alphanumeric(b'w'))
                | Event::KeyUp(Keycode::Alphanumeric(b's')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        velocity.set_z(0.0);
                        self.dirty = true;
                    }
                }
                Event::KeyUp(Keycode::Alphanumeric(b'a'))
                | Event::KeyUp(Keycode::Alphanumeric(b'd')) => {
                    if let Camera::ThreeD {
                        ref mut velocity, ..
                    } = self.camera
                    {
                        velocity.set_x(0.0);
                        self.dirty = true;
                    }
                }
                Event::KeyDown(Keycode::Tab) => {
                    self.options.ui = match self.options.ui {
                        UIVisibility::None => UIVisibility::Stats,
                        UIVisibility::Stats => UIVisibility::All,
                        UIVisibility::All => UIVisibility::None,
                    }
                }

                Event::OpenSVG(ref svg_path) => {
                    let mut built_svg = load_scene(self.window.resource_loader(), svg_path);
                    self.ui.message = get_svg_building_message(&built_svg);

                    let viewport_size = self.window.viewport(self.ui.mode.view(0)).size();
                    self.scene_metadata =
                        SceneMetadata::new_clipping_view_box(&mut built_svg.scene, viewport_size);
                    self.camera = Camera::new(self.ui.mode,
                                              self.scene_metadata.view_box,
                                              viewport_size);

                    self.scene_proxy.replace_scene(built_svg.scene);

                    self.dirty = true;
                }

                Event::User {
                    message_type: event_id,
                    message_data: expected_epoch,
                } if event_id == self.expire_message_event_id
                    && expected_epoch as u32 == self.message_epoch =>
                {
                    self.ui.message = String::new();
                    self.dirty = true;
                }
                _ => continue,
            }
        }

        ui_events
    }

    fn process_mouse_position(&mut self, new_position: Point2DI32) -> MousePosition {
        let absolute = new_position.scale(self.window_size.backing_scale_factor as i32);
        let relative = absolute - self.last_mouse_position;
        self.last_mouse_position = absolute;
        MousePosition { absolute, relative }
    }

    pub fn draw_scene(&mut self) {
        let view = self.ui.mode.view(0);
        self.window.make_current(view);

        if self.camera.mode() != Mode::VR {
            self.draw_environment();
        }

        self.render_vector_scene();

        // Reattach default framebuffer.
        if self.camera.mode() != Mode::VR {
            return;
        }

        if let DestFramebuffer::Other(scene_framebuffer) =
            self.renderer
                .replace_dest_framebuffer(DestFramebuffer::Default {
                    viewport: self.window.viewport(View::Mono),
                    window_size: self.window_size.device_size(),
                })
        {
            self.scene_framebuffer = Some(scene_framebuffer);
        }
    }

    pub fn composite_scene(&mut self, render_scene_index: u32) {
        let (eye_transforms, scene_transform, modelview_transform) = match self.camera {
            Camera::ThreeD {
                ref eye_transforms,
                ref scene_transform,
                ref modelview_transform,
                ..
            } if eye_transforms.len() > 1 => (eye_transforms, scene_transform, modelview_transform),
            _ => return,
        };

        debug!(
            "scene_transform.perspective={:?}",
            scene_transform.perspective
        );
        debug!(
            "scene_transform.modelview_to_eye={:?}",
            scene_transform.modelview_to_eye
        );
        debug!("modelview transform={:?}", modelview_transform);

        let viewport = self.window.viewport(View::Stereo(render_scene_index));
        self.renderer
            .replace_dest_framebuffer(DestFramebuffer::Default {
                viewport,
                window_size: self.window_size.device_size(),
            });

        self.renderer.bind_draw_framebuffer();
        self.renderer.device.clear(&ClearParams {
            color: Some(self.background_color().to_f32()),
            depth: Some(1.0),
            stencil: Some(0),
            rect: Some(viewport),
        });

        self.draw_environment();

        let scene_framebuffer = self.scene_framebuffer.as_ref().unwrap();
        let scene_texture = self.renderer.device.framebuffer_texture(scene_framebuffer);

        let quad_scale_transform = Transform3DF32::from_scale(
            self.scene_metadata.view_box.size().x(),
            self.scene_metadata.view_box.size().y(),
            1.0,
        );

        let scene_transform_matrix = scene_transform
            .perspective
            .post_mul(&scene_transform.modelview_to_eye)
            .post_mul(&modelview_transform.to_transform())
            .post_mul(&quad_scale_transform);

        let eye_transform = &eye_transforms[render_scene_index as usize];
        let eye_transform_matrix = eye_transform
            .perspective
            .post_mul(&eye_transform.modelview_to_eye)
            .post_mul(&modelview_transform.to_transform())
            .post_mul(&quad_scale_transform);

        debug!(
            "eye transform({}).modelview_to_eye={:?}",
            render_scene_index, eye_transform.modelview_to_eye
        );
        debug!(
            "eye transform_matrix({})={:?}",
            render_scene_index, eye_transform_matrix
        );
        debug!("---");

        self.renderer.reproject_texture(
            scene_texture,
            &scene_transform_matrix.transform,
            &eye_transform_matrix.transform,
        );
    }

    pub fn finish_drawing_frame(&mut self) {
        if let Some(rendering_time) = self.renderer.shift_timer_query() {
            self.current_frame
                .as_mut()
                .unwrap()
                .scene_rendering_times
                .push(rendering_time);
        }

        let build_time = self.build_time.unwrap();
        let mut frame = self.current_frame.take().unwrap();

        if self.pending_screenshot_path.is_some() {
            self.take_screenshot();
        }

        if !frame.scene_stats.is_empty() || !frame.scene_rendering_times.is_empty() {
            let zero = RenderStats::default();
            let aggregate_stats = frame.scene_stats.iter().fold(zero, |sum, item| sum + *item);
            let total_rendering_time = if frame.scene_rendering_times.is_empty() {
                None
            } else {
                let zero = Duration::new(0, 0);
                Some(
                    frame
                        .scene_rendering_times
                        .iter()
                        .fold(zero, |sum, item| sum + *item),
                )
            };
            self.renderer
                .debug_ui
                .add_sample(aggregate_stats, build_time, total_rendering_time);
        }

        if self.options.ui != UIVisibility::None {
            let viewport = self.window.viewport(View::Mono);
            self.window.make_current(View::Mono);
            self.renderer
                .replace_dest_framebuffer(DestFramebuffer::Default {
                    viewport,
                    window_size: self.window_size.device_size(),
                });
            self.renderer.draw_debug_ui();
        }

        for ui_event in &frame.ui_events {
            self.dirty = true;
            self.renderer.debug_ui.ui.event_queue.push(*ui_event);
        }

        self.renderer.debug_ui.ui.mouse_position = self
            .last_mouse_position
            .to_f32()
            .scale(self.window_size.backing_scale_factor);
        self.ui.show_text_effects = self.scene_metadata.monochrome_color.is_some();

        let mut ui_action = UIAction::None;
        if self.options.ui == UIVisibility::All {
            self.ui.update(
                &self.renderer.device,
                &mut self.window,
                &mut self.renderer.debug_ui,
                &mut ui_action,
            );
        }
        frame.ui_events = self.renderer.debug_ui.ui.event_queue.drain();
        self.handle_ui_action(&mut ui_action);

        // Switch camera mode (2D/3D) if requested.
        //
        // FIXME(pcwalton): This should really be an MVC setup.
        if self.camera.mode() != self.ui.mode {
            let viewport_size = self.window.viewport(self.ui.mode.view(0)).size();
            self.camera = Camera::new(self.ui.mode, self.scene_metadata.view_box, viewport_size);
        }

        for ui_event in frame.ui_events {
            match ui_event {
                UIEvent::MouseDown(_) if self.camera.is_3d() => {
                    // If nothing handled the mouse-down event, toggle mouselook.
                    self.mouselook_enabled = !self.mouselook_enabled;
                }
                UIEvent::MouseDragged(position) => {
                    if let Camera::TwoD(ref mut transform) = self.camera {
                        *transform = transform.post_translate(position.relative.to_f32());
                    }
                }
                _ => {}
            }
        }

        self.window.present();
        self.frame_counter += 1;
    }

    fn draw_environment(&self) {
        // TODO(pcwalton): Use the viewport index!

        let frame = &self.current_frame.as_ref().unwrap();

        let perspective = match frame.transform {
            RenderTransform::Transform2D(..) => return,
            RenderTransform::Perspective(perspective) => perspective,
        };

        if self.ui.background_color == BackgroundColor::Transparent {
            return;
        }

        let ground_scale = self.scene_metadata.view_box.max_x() * 2.0;

        let mut base_transform = perspective.transform;
        base_transform = base_transform.post_mul(&Transform3DF32::from_translation(
            -0.5 * self.scene_metadata.view_box.max_x(),
            self.scene_metadata.view_box.max_y(),
            -0.5 * ground_scale,
        ));

        // Fill ground.
        let mut transform = base_transform;
        transform =
            transform.post_mul(&Transform3DF32::from_scale(ground_scale, 1.0, ground_scale));

        let device = &self.renderer.device;
        device.bind_vertex_array(&self.ground_vertex_array.vertex_array);
        device.use_program(&self.ground_program.program);
        device.set_uniform(
            &self.ground_program.transform_uniform,
            UniformData::from_transform_3d(&transform),
        );
        device.set_uniform(
            &self.ground_program.ground_color_uniform,
            UniformData::Vec4(GROUND_SOLID_COLOR.to_f32().0),
        );
        device.set_uniform(
            &self.ground_program.gridline_color_uniform,
            UniformData::Vec4(GROUND_LINE_COLOR.to_f32().0),
        );
        device.set_uniform(&self.ground_program.gridline_count_uniform,
                           UniformData::Int(GRIDLINE_COUNT));
        device.draw_arrays(
            Primitive::TriangleFan,
            4,
            &RenderState {
                depth: Some(DepthState {
                    func: DepthFunc::Less,
                    write: true,
                }),
                stencil: None,
                ..RenderState::default()
            },
        );
    }

    fn render_vector_scene(&mut self) {
        match self.scene_metadata.monochrome_color {
            None => self.renderer.set_render_mode(RenderMode::Multicolor),
            Some(fg_color) => {
                self.renderer.set_render_mode(RenderMode::Monochrome {
                    fg_color: fg_color.to_f32(),
                    bg_color: self.background_color().to_f32(),
                    gamma_correction: self.ui.gamma_correction_effect_enabled,
                    defringing_kernel: if self.ui.subpixel_aa_effect_enabled {
                        // TODO(pcwalton): Select FreeType defringing kernel as necessary.
                        Some(DEFRINGING_KERNEL_CORE_GRAPHICS)
                    } else {
                        None
                    },
                })
            }
        }

        if self.ui.mode == Mode::TwoD {
            self.renderer.disable_depth();
        } else {
            self.renderer.enable_depth();
        }

        self.renderer.begin_scene();

        // Issue render commands!
        for command in self.render_command_stream.as_mut().unwrap() {
            self.renderer.render_command(&command);

            if let RenderCommand::Finish { build_time } = command {
                self.build_time = Some(build_time);
            }
        }

        self.current_frame
            .as_mut()
            .unwrap()
            .scene_stats
            .push(self.renderer.stats);
        self.renderer.end_scene();
    }

    fn handle_ui_action(&mut self, ui_action: &mut UIAction) {
        match ui_action {
            UIAction::None => {}
            UIAction::ModelChanged => self.dirty = true,
            UIAction::TakeScreenshot(ref path) => {
                self.pending_screenshot_path = Some((*path).clone());
                self.dirty = true;
            }
            UIAction::ZoomIn => {
                if let Camera::TwoD(ref mut transform) = self.camera {
                    let scale = Point2DF32::splat(1.0 + CAMERA_ZOOM_AMOUNT_2D);
                    let center = center_of_window(&self.window_size);
                    *transform = transform
                        .post_translate(-center)
                        .post_scale(scale)
                        .post_translate(center);
                    self.dirty = true;
                }
            }
            UIAction::ZoomOut => {
                if let Camera::TwoD(ref mut transform) = self.camera {
                    let scale = Point2DF32::splat(1.0 - CAMERA_ZOOM_AMOUNT_2D);
                    let center = center_of_window(&self.window_size);
                    *transform = transform
                        .post_translate(-center)
                        .post_scale(scale)
                        .post_translate(center);
                    self.dirty = true;
                }
            }
            UIAction::Rotate(theta) => {
                if let Camera::TwoD(ref mut transform) = self.camera {
                    let old_rotation = transform.rotation();
                    let center = center_of_window(&self.window_size);
                    *transform = transform
                        .post_translate(-center)
                        .post_rotate(*theta - old_rotation)
                        .post_translate(center);
                }
            }
        }
    }

    fn take_screenshot(&mut self) {
        let screenshot_path = self.pending_screenshot_path.take().unwrap();
        let drawable_size = self.window_size.device_size();
        let pixels = self
            .renderer
            .device
            .read_pixels_from_default_framebuffer(drawable_size);
        image::save_buffer(
            screenshot_path,
            &pixels,
            drawable_size.x() as u32,
            drawable_size.y() as u32,
            ColorType::RGBA(8),
        )
        .unwrap();
    }

    fn background_color(&self) -> ColorU {
        match self.ui.background_color {
            BackgroundColor::Light => LIGHT_BG_COLOR,
            BackgroundColor::Dark => DARK_BG_COLOR,
            BackgroundColor::Transparent => TRANSPARENT_BG_COLOR,
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub jobs: Option<usize>,
    pub mode: Mode,
    pub input_path: SVGPath,
    pub ui: UIVisibility,
    pub background_color: BackgroundColor,
    hidden_field_for_future_proofing: (),
}

impl Default for Options {
    fn default() -> Self {
        Options {
            jobs: None,
            mode: Mode::TwoD,
            input_path: SVGPath::Default,
            ui: UIVisibility::All,
            background_color: BackgroundColor::Light,
            hidden_field_for_future_proofing: (),
        }
    }
}

impl Options {
    fn command_line_overrides(&mut self) {
        let matches = App::new("tile-svg")
            .arg(
                Arg::with_name("jobs")
                    .short("j")
                    .long("jobs")
                    .value_name("THREADS")
                    .takes_value(true)
                    .help("Number of threads to use"),
            )
            .arg(
                Arg::with_name("3d")
                    .short("3")
                    .long("3d")
                    .help("Run in 3D")
                    .conflicts_with("vr"),
            )
            .arg(
                Arg::with_name("vr")
                    .short("V")
                    .long("vr")
                    .help("Run in VR")
                    .conflicts_with("3d"),
            )
            .arg(
                Arg::with_name("ui")
                    .short("u")
                    .long("ui")
                    .takes_value(true)
                    .possible_values(&["none", "stats", "all"])
                    .help("How much UI to show"),
            )
            .arg(
                Arg::with_name("background")
                    .short("b")
                    .long("background")
                    .takes_value(true)
                    .possible_values(&["light", "dark", "transparent"])
                    .help("The background color to use"),
            )
            .arg(
                Arg::with_name("INPUT")
                    .help("Path to the SVG file to render")
                    .index(1),
            )
            .get_matches();

        if let Some(jobs) = matches.value_of("jobs") {
            self.jobs = jobs.parse().ok();
        }

        if matches.is_present("3d") {
            self.mode = Mode::ThreeD;
        } else if matches.is_present("vr") {
            self.mode = Mode::VR;
        }

        if let Some(ui) = matches.value_of("ui") {
            self.ui = match ui {
                "none" => UIVisibility::None,
                "stats" => UIVisibility::Stats,
                _ => UIVisibility::All,
            };
        }

        if let Some(background_color) = matches.value_of("background") {
            self.background_color = match background_color {
                "light" => BackgroundColor::Light,
                "dark" => BackgroundColor::Dark,
                _ => BackgroundColor::Transparent,
            };
        }

        if let Some(path) = matches.value_of("INPUT") {
            self.input_path = SVGPath::Path(PathBuf::from(path));
        };
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum UIVisibility {
    None,
    Stats,
    All,
}

fn load_scene(resource_loader: &dyn ResourceLoader, input_path: &SVGPath) -> BuiltSVG {
    let mut data;
    match *input_path {
        SVGPath::Default => data = resource_loader.slurp(DEFAULT_SVG_VIRTUAL_PATH).unwrap(),
        SVGPath::Resource(ref name) => data = resource_loader.slurp(name).unwrap(),
        SVGPath::Path(ref path) => {
            data = vec![];
            File::open(path).unwrap().read_to_end(&mut data).unwrap();
        }
    };

    BuiltSVG::from_tree(Tree::from_data(&data, &UsvgOptions::default()).unwrap())
}

fn center_of_window(window_size: &WindowSize) -> Point2DF32 {
    window_size.device_size().to_f32().scale(0.5)
}

fn get_svg_building_message(built_svg: &BuiltSVG) -> String {
    if built_svg.result_flags.is_empty() {
        return String::new();
    }
    format!(
        "Warning: These features in the SVG are unsupported: {}.",
        built_svg.result_flags
    )
}

fn emit_message<W>(
    ui: &mut DemoUI<GLDevice>,
    message_epoch: &mut u32,
    expire_message_event_id: u32,
    message: String,
) where
    W: Window,
{
    if message.is_empty() {
        return;
    }

    ui.message = message;
    let expected_epoch = *message_epoch + 1;
    *message_epoch = expected_epoch;
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(MESSAGE_TIMEOUT_SECS));
        W::push_user_event(expire_message_event_id, expected_epoch);
    });
}

struct Frame {
    transform: RenderTransform,
    ui_events: Vec<UIEvent>,
    scene_rendering_times: Vec<Duration>,
    scene_stats: Vec<RenderStats>,
}

impl Frame {
    fn new(transform: RenderTransform, ui_events: Vec<UIEvent>) -> Frame {
        Frame {
            transform,
            ui_events,
            scene_rendering_times: vec![],
            scene_stats: vec![],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackgroundColor {
    Light = 0,
    Dark = 1,
    Transparent = 2,
}

impl BackgroundColor {
    fn as_str(&self) -> &'static str {
        match *self {
            BackgroundColor::Light => "Light",
            BackgroundColor::Dark => "Dark",
            BackgroundColor::Transparent => "Transparent",
        }
    }
}

struct SceneMetadata {
    view_box: RectF32,
    bounds: RectF32,
    monochrome_color: Option<ColorU>,
}

impl SceneMetadata {
    // FIXME(pcwalton): The fact that this mutates the scene is really ugly!
    // Can we simplify this?
    fn new_clipping_view_box(scene: &mut Scene, viewport_size: Point2DI32) -> SceneMetadata {
        let view_box = scene.view_box();
        let monochrome_color = scene.monochrome_color();
        scene.set_view_box(RectF32::new(Point2DF32::default(), viewport_size.to_f32()));
        SceneMetadata { view_box, monochrome_color, bounds: scene.bounds() }
    }
}