use bevy_math::Vec2;
use egui::{
    Color32, Frame, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, WidgetText, emath,
    epaint::CubicBezierShape,
};
use std::collections::HashSet;

use crate::{Knot, KnotInterpolation, LookupCurve, TangentMode, TangentSide};

const DBFS_FLOOR_DB: f32 = -60.0;

#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
/// Lookup curve editor implemented using `egui`.
///
/// Holds the editor state.
pub struct LookupCurveEguiEditor {
    pub offset: Vec2,
    pub scale: Vec2,
    pub bipolar: bool,
    pub dbfs_view: bool,
    pub dbfs_axis_only: bool,
    pub dbfs_x_axis_labels: bool,
    pub strict_dbfs_ticks: bool,

    pub grid_step_x: f32,
    pub grid_step_y: f32,

    pub editor_size: Vec2,
    pub hover_point: Vec2,

    pub sample_point: Option<Pos2>,
    pub top_one_point: Pos2,
    pub bottom_zero_point: Pos2,
    pub has_focus: bool,
    pub selected_knot_ids: HashSet<usize>,
    pub marquee_start: Option<Pos2>,
    pub marquee_current: Option<Pos2>,
    pub context_menu_extension: Option<Box<dyn FnMut(&mut Ui, &Knot) + Send>>,

    #[cfg(feature = "ron")]
    pub ron_path: Option<String>,
}

impl Default for LookupCurveEguiEditor {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            bipolar: false,
            dbfs_view: false,
            dbfs_axis_only: false,
            dbfs_x_axis_labels: false,
            strict_dbfs_ticks: false,

            grid_step_x: 0.1,
            grid_step_y: 0.1,

            editor_size: Vec2::ZERO,
            hover_point: Vec2::ZERO,

            sample_point: None,
            top_one_point: Pos2::default(),
            bottom_zero_point: Pos2::default(),
            has_focus: false,
            selected_knot_ids: HashSet::new(),
            marquee_start: None,
            marquee_current: None,
            context_menu_extension: None,

            #[cfg(feature = "ron")]
            ron_path: None,
        }
    }
}

impl LookupCurveEguiEditor {
    /// Constructs a [LookupCurveEguiEditor] with the supplied `path` as save path.
    #[cfg(feature = "ron")]
    pub fn with_save_path(path: String) -> Self {
        Self {
            ron_path: Some(path),
            ..Default::default()
        }
    }

    /// Constructs a [LookupCurveEguiEditor] with the viewport adjusted to fit the supplied [LookupCurve].
    pub fn fitted_to_curve(curve: &LookupCurve) -> Self {
        let mut editor = LookupCurveEguiEditor::default();
        editor.fit_to_curve(curve);
        editor
    }

    /// Fits the editor viewport to the supplied [LookupCurve] by updating scale and offset.
    pub fn fit_to_curve(&mut self, curve: &LookupCurve) {
        let knots = curve.knots();
        let (min, max) = match knots.len() {
            0 => (Vec2::ZERO, Vec2::ONE),
            1 => {
                let pos = knots[0].position;
                let padding = Vec2::splat(0.5);
                (pos - padding, pos + padding)
            }
            _ => knots
                .iter()
                .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(min, max), knot| {
                    (min.min(knot.position), max.max(knot.position))
                }),
        };
        let diff = max - min;

        self.offset = min - 0.2 * diff;
        self.scale = diff * 1.4;
    }

    pub fn configure_dbfs_mode(
        &mut self,
        dbfs_view: bool,
        dbfs_axis_only: bool,
        dbfs_x_axis_labels: bool,
        strict_dbfs_ticks: bool,
    ) {
        self.dbfs_view = dbfs_view;
        self.dbfs_axis_only = dbfs_axis_only;
        self.dbfs_x_axis_labels = dbfs_x_axis_labels;
        self.strict_dbfs_ticks = strict_dbfs_ticks;
    }

    // TODO : Rename these functions and make them clearer
    // Move to a paintcontext? with access to to_screeen / to_canvas

    fn curve_to_canvas(&self, curve: Vec2) -> Pos2 {
        let canvas = (curve - self.offset) * self.editor_size / self.scale;
        Pos2::new(canvas.x, self.editor_size.y - canvas.y)
    }

    fn db_to_curve_axis(&self, db: f32, bipolar: bool) -> f32 {
        let normalized = dbfs_label_value_to_axis(db);
        if bipolar && db <= 0.0 {
            -normalized
        } else {
            normalized
        }
    }

    fn strict_db_tick_values(&self) -> Vec<f32> {
        let side_db_ticks = [
            0.0_f32, -6.0, -12.0, -18.0, -24.0, -30.0, -36.0, -42.0, -48.0, -54.0,
        ];
        let mut values = Vec::new();

        if self.bipolar {
            for db in side_db_ticks.iter().copied() {
                values.push(self.db_to_curve_axis(db, true));
            }
            values.push(0.0);
        } else {
            values.push(0.0);
        }

        for db in side_db_ticks.iter().rev().copied() {
            values.push(self.db_to_curve_axis(db, false));
        }

        values
    }

    fn filter_visible_tick_values(
        &self,
        values: Vec<f32>,
        axis_is_x: bool,
        min_value: f32,
        max_value: f32,
        min_pixel_spacing: f32,
    ) -> Vec<f32> {
        let mut filtered = Vec::new();
        let mut last_pixel: Option<f32> = None;

        for value in values {
            if value < min_value - 1.0e-6 || value > max_value + 1.0e-6 {
                continue;
            }

            let canvas_pos = if axis_is_x {
                self.curve_to_canvas(Vec2::new(value, self.offset.y)).x
            } else {
                self.curve_to_canvas(Vec2::new(self.offset.x, value)).y
            };

            if let Some(last_pixel) = last_pixel
                && (canvas_pos - last_pixel).abs() < min_pixel_spacing
            {
                continue;
            }

            filtered.push(value);
            last_pixel = Some(canvas_pos);
        }

        filtered
    }

    fn curve_to_canvas_vec(&self, curve: Vec2) -> emath::Vec2 {
        let origin = self.curve_to_canvas(Vec2::ZERO);
        let endpoint = self.curve_to_canvas(curve);
        endpoint - origin
    }

    fn canvas_to_curve(&self, canvas: Pos2) -> Vec2 {
        let canvas = Vec2::new(canvas.x, self.editor_size.y - canvas.y);
        canvas * self.scale / self.editor_size + self.offset
    }

    pub fn curve_to_screen(&self, rect: Rect, curve: Vec2) -> Pos2 {
        let canvas_pos = self.curve_to_canvas(curve);
        Pos2::new(rect.left() + canvas_pos.x, rect.top() + canvas_pos.y)
    }

    fn point_hits_interactive(
        &self,
        curve: &LookupCurve,
        to_screen: &emath::RectTransform,
        pointer_pos: Pos2,
        knot_radius: f32,
    ) -> bool {
        let hit_rect = |screen_pos: Pos2| {
            Rect::from_center_size(screen_pos, emath::Vec2::splat(2.0 * knot_radius))
                .contains(pointer_pos)
        };

        for (i, knot) in curve.knots().iter().enumerate() {
            let point_in_screen = to_screen.transform_pos(self.curve_to_canvas(knot.position));
            if hit_rect(point_in_screen) {
                return true;
            }

            let prev_knot = curve.prev_knot(i);
            let next_knot = curve.next_knot(i);
            const UNWEIGHTED_TANGENT_LEN: f32 = 60.;

            let tangent_point = |side: TangentSide| -> Option<Pos2> {
                let (tangent, bezier, dir) = match side {
                    TangentSide::Left => {
                        (knot.left_tangent, prev_knot?.compute_bezier_to(knot), -1.)
                    }
                    TangentSide::Right => {
                        (knot.right_tangent, knot.compute_bezier_to(next_knot?), 1.)
                    }
                };
                let (_endpoint, intermediate) = match side {
                    TangentSide::Left => (bezier[3], bezier[2]),
                    TangentSide::Right => (bezier[0], bezier[1]),
                };
                let point_in_canvas = if tangent.weight.is_some() {
                    self.curve_to_canvas(intermediate)
                } else {
                    let tangent_vec = self.curve_to_canvas_vec(intermediate - knot.position);
                    let normalized = if tangent_vec.is_finite() && tangent_vec.length_sq() > 1.0e-12
                    {
                        tangent_vec.normalized()
                    } else {
                        emath::Vec2::new(dir, 0.0)
                    };
                    self.curve_to_canvas(knot.position) + normalized * UNWEIGHTED_TANGENT_LEN
                };

                let point_in_screen = to_screen.transform_pos(point_in_canvas);
                point_in_screen.is_finite().then_some(point_in_screen)
            };

            if matches!(knot.interpolation, KnotInterpolation::Cubic)
                && let Some(point) = tangent_point(TangentSide::Right)
                && hit_rect(point)
            {
                return true;
            }

            if prev_knot.is_some()
                && matches!(prev_knot.unwrap().interpolation, KnotInterpolation::Cubic)
                && let Some(point) = tangent_point(TangentSide::Left)
                && hit_rect(point)
            {
                return true;
            }
        }

        false
    }

    fn apply_interpolation_to_selection(
        &self,
        curve: &mut LookupCurve,
        selected_ids: &[usize],
        interpolation: KnotInterpolation,
    ) -> bool {
        let mut changed = false;
        for knot_id in selected_ids {
            if let Some(idx) = curve.knots().iter().position(|knot| knot.id == *knot_id) {
                let original = curve.knots()[idx];
                if !matches!(
                    (original.interpolation, interpolation),
                    (KnotInterpolation::Constant, KnotInterpolation::Constant)
                        | (KnotInterpolation::Linear, KnotInterpolation::Linear)
                        | (KnotInterpolation::Cubic, KnotInterpolation::Cubic)
                ) {
                    curve.modify_knot(
                        idx,
                        Knot {
                            interpolation,
                            ..original
                        },
                    );
                    changed = true;
                }
            }
        }
        changed
    }

    fn delete_selected_knots(&mut self, curve: &mut LookupCurve) -> bool {
        let mut selected_indices: Vec<_> = curve
            .knots()
            .iter()
            .enumerate()
            .filter(|(_, knot)| self.selected_knot_ids.contains(&knot.id))
            .map(|(idx, _)| idx)
            .collect();
        if selected_indices.is_empty() {
            return false;
        }

        selected_indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in selected_indices {
            curve.delete_knot(idx);
        }
        self.selected_knot_ids.clear();
        true
    }

    /// Display the editor in a window
    ///
    /// If a `sample` is supplied, it will be displayed as a red dot on the curve.
    ///
    /// Returns `true` if the curve was changed during this update
    pub fn ui_window(
        &mut self,
        ctx: &mut egui::Context,
        id: impl std::hash::Hash,
        title: impl Into<WidgetText>,
        curve: &mut LookupCurve,
        sample: Option<f32>,
    ) -> bool {
        let mut changed = false;
        egui::Window::new(title).id(Id::new(id)).show(ctx, |ui| {
            changed = self.ui(ui, curve, sample);
        });
        changed
    }

    /// Display the editor
    ///
    /// If a `sample` is supplied, it will be displayed as a red dot on the curve.
    ///
    /// Returns `true` if the curve was changed during this update
    pub fn ui(&mut self, ui: &mut Ui, curve: &mut LookupCurve, sample: Option<f32>) -> bool {
        let mut changed = false;
        // ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        //     if ui.button("Refocus curve").clicked() {
        //         changed = true;
        //         self.fit_to_curve(curve);
        //     }

        //     ui.label(format!(
        //         "x = {:.2}, y = {:.2}",
        //         self.hover_point.x, self.hover_point.y
        //     ));
        // });

        #[cfg(feature = "ron")]
        if let Some(ron_path) = self.ron_path.as_ref()
            && ui.button("Save").clicked()
        {
            if let Err(e) = curve.save_to_file(ron_path.as_str()) {
                #[cfg(feature = "bevy_app")]
                bevy_log::error!("Failed to save curve {}", e);
                #[cfg(not(feature = "bevy_app"))]
                println!("Failed to save curve {}", e);
            } else {
                #[cfg(feature = "bevy_app")]
                bevy_log::info!("Curve saved successfully.");
                #[cfg(not(feature = "bevy_app"))]
                println!("Curve saved successfully.");
            }
        }

        Frame::new().show(ui, |ui| {
            let (response, painter) = ui.allocate_painter(
                emath::Vec2::new(ui.available_width(), ui.available_height()),
                Sense::click_and_drag(),
            );

            let to_screen = emath::RectTransform::from_to(
                Rect::from_min_size(Pos2::ZERO, response.rect.size()),
                response.rect,
            );
            let to_canvas = emath::RectTransform::from_to(
                response.rect,
                Rect::from_min_size(Pos2::ZERO, response.rect.size()),
            );

            let width = response.rect.width();
            let height = response.rect.height();
            self.editor_size = Vec2::new(width, height);
            let knot_radius = 8.0;
            let additive_selection = ui.input(|i| i.modifiers.shift);
            let pointer_over_interactive = response
                .interact_pointer_pos()
                .map(|pointer_pos| {
                    self.point_hits_interactive(curve, &to_screen, pointer_pos, knot_radius)
                })
                .unwrap_or(false);
            let mut selection_interpolation_change: Option<(Vec<usize>, KnotInterpolation)> = None;
            let mut delete_selection_from_menu = false;

            if let Some(hover_pos) = response.hover_pos() {
                self.hover_point = self.canvas_to_curve(to_canvas.transform_pos(hover_pos));

                // Zooming
                ui.input(|input| {
                    let scroll_delta = input.raw_scroll_delta.y;
                    if scroll_delta != 0.0 {
                        self.scale *= 1.0 + -scroll_delta * 0.001;
                    }
                });
            } else {
                self.hover_point = Vec2::ZERO;
            }

            // Panning
            if response.dragged_by(egui::PointerButton::Middle) {
                let center = Pos2::new(self.editor_size.x * 0.5, self.editor_size.y * 0.5);
                let start = self.canvas_to_curve(center);
                let end = self.canvas_to_curve(center + response.drag_delta());
                self.offset -= end - start;
            }

            if response.clicked_by(egui::PointerButton::Primary)
                || response.drag_started_by(egui::PointerButton::Primary)
            {
                response.request_focus();
            }

            if response.clicked_by(egui::PointerButton::Primary) && !pointer_over_interactive {
                self.selected_knot_ids.clear();
            }

            if response.drag_started_by(egui::PointerButton::Primary) && !pointer_over_interactive {
                let pointer_pos = response
                    .interact_pointer_pos()
                    .unwrap_or(response.rect.left_top());
                self.marquee_start = Some(pointer_pos);
                self.marquee_current = Some(pointer_pos);
                if !additive_selection {
                    self.selected_knot_ids.clear();
                }
            }

            if self.marquee_start.is_some() && response.dragged_by(egui::PointerButton::Primary) {
                self.marquee_current = response.interact_pointer_pos();
            }

            if self.marquee_start.is_some()
                && response.drag_stopped_by(egui::PointerButton::Primary)
            {
                if let (Some(start), Some(end)) = (self.marquee_start, self.marquee_current) {
                    let selection_rect = Rect::from_two_pos(start, end);
                    if selection_rect.width() > 2.0 || selection_rect.height() > 2.0 {
                        for knot in curve.knots().iter() {
                            let point_in_screen =
                                to_screen.transform_pos(self.curve_to_canvas(knot.position));
                            if selection_rect.contains(point_in_screen) {
                                self.selected_knot_ids.insert(knot.id);
                            }
                        }
                    } else if !additive_selection {
                        self.selected_knot_ids.clear();
                    }
                }

                self.marquee_start = None;
                self.marquee_current = None;
            }

            response.context_menu(|ui| {
                let menu_pos = ui.min_rect().left_top(); // hacky and not entirely correct
                if ui.button("Add knot").clicked() {
                    curve.add_knot(Knot {
                        position: self.canvas_to_curve(to_canvas.transform_pos(menu_pos)),
                        ..Default::default()
                    });
                    changed = true;
                    ui.close_menu();
                }
            });

            self.paint_grid(&painter, &to_screen);

            // Draw the curve
            let curve_stroke = Stroke {
                color: Color32::from_hex("#FFEAD0").unwrap(),
                width: 2.0,
            };

            // TODO: Only knots inside viewport
            let mut prev_knot: Option<&Knot> = None;
            for knot in curve.knots().iter() {
                if let Some(prev_knot) = prev_knot {
                    match prev_knot.interpolation {
                        KnotInterpolation::Constant => {
                            painter.add(Shape::line(
                                vec![
                                    to_screen
                                        .transform_pos(self.curve_to_canvas(prev_knot.position)),
                                    to_screen.transform_pos(self.curve_to_canvas(Vec2::new(
                                        knot.position.x,
                                        prev_knot.position.y,
                                    ))),
                                    to_screen.transform_pos(self.curve_to_canvas(knot.position)),
                                ],
                                curve_stroke,
                            ));
                        }
                        KnotInterpolation::Linear => {
                            painter.add(Shape::line(
                                vec![
                                    to_screen
                                        .transform_pos(self.curve_to_canvas(prev_knot.position)),
                                    to_screen.transform_pos(self.curve_to_canvas(knot.position)),
                                ],
                                curve_stroke,
                            ));
                        }
                        KnotInterpolation::Cubic => {
                            painter.add(CubicBezierShape::from_points_stroke(
                                prev_knot
                                    .compute_bezier_to(knot)
                                    .map(|p| to_screen.transform_pos(self.curve_to_canvas(p))),
                                false,
                                Color32::TRANSPARENT,
                                curve_stroke,
                            ));
                        }
                    }
                }

                prev_knot = Some(knot);
            }

            // Handles
            let mut modified_knot = None;
            let mut move_selected_by: Option<(Vec<usize>, Vec2)> = None;
            let mut deleted_knot_index = None;
            let delete_selected_requested = response.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                })
                && !self.selected_knot_ids.is_empty();
            for (i, knot) in curve.knots().iter().enumerate() {
                let prev_knot = curve.prev_knot(i);
                let next_knot = curve.next_knot(i);

                let point_in_screen = to_screen.transform_pos(self.curve_to_canvas(knot.position));
                let interact_rect =
                    Rect::from_center_size(point_in_screen, emath::Vec2::splat(2.0 * knot_radius));
                let interact_id = response.id.with(knot.id);
                let interact_response =
                    ui.interact(interact_rect, interact_id, Sense::click_and_drag());

                if interact_response.clicked_by(egui::PointerButton::Primary) {
                    response.request_focus();
                    if additive_selection {
                        if !self.selected_knot_ids.insert(knot.id) {
                            self.selected_knot_ids.remove(&knot.id);
                        }
                    } else {
                        self.selected_knot_ids.clear();
                        self.selected_knot_ids.insert(knot.id);
                    }
                }

                let snap_threshold = 0.01;

                if interact_response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(pointer_pos) = interact_response.interact_pointer_pos() {
                        let mut new_pos =
                            self.canvas_to_curve(to_screen.inverse().transform_pos(pointer_pos));

                        let mods = ui.input(|i| i.modifiers);
                        let snap_targets = if self.bipolar {
                            [-1.0, -0.5, 0.0, 0.5, 1.0]
                        } else {
                            [0.0, 0.5, 1.0, 10.0, 10.0]
                        };
                        let min_x = if self.bipolar { -1.0 } else { 0.0 };
                        let min_y = if self.bipolar { -1.0 } else { 0.0 };

                        if mods.shift {
                            new_pos.x = (new_pos.x * 10.0).round() / 10.0;
                            new_pos.y = (new_pos.y * 10.0).round() / 10.0;
                        } else {
                            let snap = |val: f32| {
                                for &t in &snap_targets {
                                    if t > 1.0 {
                                        continue;
                                    }
                                    if (val - t).abs() < snap_threshold {
                                        return t;
                                    }
                                }
                                val
                            };
                            new_pos.x = snap(new_pos.x);
                            new_pos.y = snap(new_pos.y);
                        }

                        if !mods.alt {
                            new_pos.x = new_pos.x.clamp(min_x, 1.0);
                            new_pos.y = new_pos.y.clamp(min_y, 1.0);
                        }

                        if self.selected_knot_ids.contains(&knot.id) {
                            let selected_positions: Vec<_> = curve
                                .knots()
                                .iter()
                                .filter(|candidate| self.selected_knot_ids.contains(&candidate.id))
                                .map(|candidate| candidate.position)
                                .collect();

                            let mut delta = new_pos - knot.position;
                            if !selected_positions.is_empty() {
                                let selected_min_x = selected_positions
                                    .iter()
                                    .map(|pos| pos.x)
                                    .fold(f32::INFINITY, f32::min);
                                let selected_max_x = selected_positions
                                    .iter()
                                    .map(|pos| pos.x)
                                    .fold(f32::NEG_INFINITY, f32::max);
                                let selected_min_y = selected_positions
                                    .iter()
                                    .map(|pos| pos.y)
                                    .fold(f32::INFINITY, f32::min);
                                let selected_max_y = selected_positions
                                    .iter()
                                    .map(|pos| pos.y)
                                    .fold(f32::NEG_INFINITY, f32::max);

                                delta.x =
                                    delta.x.clamp(min_x - selected_min_x, 1.0 - selected_max_x);
                                delta.y =
                                    delta.y.clamp(min_y - selected_min_y, 1.0 - selected_max_y);
                            }

                            let selected_ids = curve
                                .knots()
                                .iter()
                                .filter(|candidate| self.selected_knot_ids.contains(&candidate.id))
                                .map(|candidate| candidate.id)
                                .collect();
                            move_selected_by = Some((selected_ids, delta));
                        } else {
                            modified_knot = Some((
                                i,
                                Knot {
                                    position: new_pos,
                                    ..*knot
                                },
                            ));
                        }
                    }
                }

                interact_response.context_menu(|ui| {
                    let selection_size = self.selected_knot_ids.len();
                    let batch_target_ids =
                        if self.selected_knot_ids.contains(&knot.id) && selection_size > 1 {
                            Some(self.selected_knot_ids.iter().copied().collect::<Vec<_>>())
                        } else {
                            None
                        };

                    if let Some(selected_ids) = batch_target_ids {
                        let all_constant = selected_ids.iter().all(|selected_id| {
                            curve
                                .knots()
                                .iter()
                                .find(|candidate| candidate.id == *selected_id)
                                .map(|candidate| {
                                    matches!(candidate.interpolation, KnotInterpolation::Constant)
                                })
                                .unwrap_or(false)
                        });
                        let all_linear = selected_ids.iter().all(|selected_id| {
                            curve
                                .knots()
                                .iter()
                                .find(|candidate| candidate.id == *selected_id)
                                .map(|candidate| {
                                    matches!(candidate.interpolation, KnotInterpolation::Linear)
                                })
                                .unwrap_or(false)
                        });
                        let all_cubic = selected_ids.iter().all(|selected_id| {
                            curve
                                .knots()
                                .iter()
                                .find(|candidate| candidate.id == *selected_id)
                                .map(|candidate| {
                                    matches!(candidate.interpolation, KnotInterpolation::Cubic)
                                })
                                .unwrap_or(false)
                        });

                        ui.label(format!("Selection ({selection_size} knots)"));
                        ui.separator();
                        ui.label("Interpolation");
                        if ui.radio(all_constant, "Constant").clicked() {
                            selection_interpolation_change =
                                Some((selected_ids.clone(), KnotInterpolation::Constant));
                            ui.close_menu();
                        }
                        if ui.radio(all_linear, "Linear").clicked() {
                            selection_interpolation_change =
                                Some((selected_ids.clone(), KnotInterpolation::Linear));
                            ui.close_menu();
                        }
                        if ui.radio(all_cubic, "Cubic").clicked() {
                            selection_interpolation_change =
                                Some((selected_ids.clone(), KnotInterpolation::Cubic));
                            ui.close_menu();
                        }

                        ui.separator();
                        if ui.button("Delete selected").clicked() {
                            delete_selection_from_menu = true;
                            ui.close_menu();
                        }
                    } else {
                        ui.label("Interpolation");
                        if ui
                            .radio(
                                matches!(knot.interpolation, KnotInterpolation::Constant),
                                "Constant",
                            )
                            .clicked()
                        {
                            modified_knot = Some((
                                i,
                                Knot {
                                    interpolation: KnotInterpolation::Constant,
                                    ..*knot
                                },
                            ));
                            ui.close_menu();
                        }
                        if ui
                            .radio(
                                matches!(knot.interpolation, KnotInterpolation::Linear),
                                "Linear",
                            )
                            .clicked()
                        {
                            modified_knot = Some((
                                i,
                                Knot {
                                    interpolation: KnotInterpolation::Linear,
                                    ..*knot
                                },
                            ));
                            ui.close_menu();
                        }
                        if ui
                            .radio(
                                matches!(knot.interpolation, KnotInterpolation::Cubic),
                                "Cubic",
                            )
                            .clicked()
                        {
                            modified_knot = Some((
                                i,
                                Knot {
                                    interpolation: KnotInterpolation::Cubic,
                                    ..*knot
                                },
                            ));
                            ui.close_menu();
                        }

                        ui.label("Position");
                        ui.horizontal(|ui| {
                            ui.label("x:");
                            ui.add(
                                egui::DragValue::from_get_set(|v| match v {
                                    Some(v) => {
                                        modified_knot = Some((
                                            i,
                                            Knot {
                                                position: Vec2::new(v as f32, knot.position.y),
                                                ..*knot
                                            },
                                        ));
                                        v
                                    }
                                    _ => knot.position.x as f64,
                                })
                                .speed(0.001),
                            );
                            ui.label("y:");
                            ui.add(
                                egui::DragValue::from_get_set(|v| match v {
                                    Some(v) => {
                                        modified_knot = Some((
                                            i,
                                            Knot {
                                                position: Vec2::new(knot.position.x, v as f32),
                                                ..*knot
                                            },
                                        ));
                                        v
                                    }
                                    _ => knot.position.y as f64,
                                })
                                .speed(0.001),
                            );
                        });

                        ui.label("Actions");
                        if let Some(extension) = self.context_menu_extension.as_mut() {
                            ui.separator();
                            extension(ui, knot);
                        }
                        if ui.button("Delete knot").clicked() {
                            deleted_knot_index = Some(i);
                            ui.close_menu();
                        }
                    }
                });

                painter.add(Shape::circle_filled(
                    to_screen.transform_pos(self.curve_to_canvas(knot.position)),
                    3.0,
                    Color32::from_hex("#DB9160").unwrap(),
                ));

                if self.selected_knot_ids.contains(&knot.id) {
                    painter.add(Shape::circle_stroke(
                        to_screen.transform_pos(self.curve_to_canvas(knot.position)),
                        6.0,
                        Stroke::new(1.5, Color32::from_hex("#FFEAD0").unwrap()),
                    ));
                }

                // tangents
                const UNWEIGHTED_TANGENT_LEN: f32 = 60.;
                let mut tangent_ui = |side: TangentSide| {
                    let (tangent, bezier, dir) = match side {
                        TangentSide::Left => (
                            knot.left_tangent,
                            prev_knot.unwrap().compute_bezier_to(knot),
                            -1.,
                        ),
                        TangentSide::Right => (
                            knot.right_tangent,
                            knot.compute_bezier_to(next_knot.unwrap()),
                            1.,
                        ),
                    };
                    let (endpoint, intermediate) = match side {
                        TangentSide::Left => (bezier[3], bezier[2]),
                        TangentSide::Right => (bezier[0], bezier[1]),
                    };
                    let point_in_canvas = if tangent.weight.is_some() {
                        self.curve_to_canvas(intermediate)
                    } else {
                        let tangent_vec = self.curve_to_canvas_vec(intermediate - knot.position);
                        let normalized =
                            if tangent_vec.is_finite() && tangent_vec.length_sq() > 1.0e-12 {
                                tangent_vec.normalized()
                            } else {
                                emath::Vec2::new(dir, 0.0)
                            };
                        self.curve_to_canvas(knot.position) + normalized * UNWEIGHTED_TANGENT_LEN
                    };

                    let point_in_screen = to_screen.transform_pos(point_in_canvas);
                    if !point_in_screen.is_finite() {
                        return;
                    }

                    let interact_rect = Rect::from_center_size(
                        point_in_screen,
                        emath::Vec2::splat(2.0 * knot_radius),
                    );
                    let interact_id = interact_id.with(side);
                    let interact_response =
                        ui.interact(interact_rect, interact_id, Sense::click_and_drag());

                    if interact_response.dragged_by(egui::PointerButton::Primary) {
                        let mut c = self.canvas_to_curve(
                            to_canvas
                                .transform_pos(interact_response.interact_pointer_pos().unwrap()),
                        );

                        // Clamp tangent to its side on the x-axis
                        c = match side {
                            TangentSide::Left => c.with_x(c.x.min(knot.position.x - f32::EPSILON)),
                            TangentSide::Right => c.with_x(c.x.max(knot.position.x + f32::EPSILON)),
                        };

                        if tangent.weight.is_some() {
                            let dx = (bezier[3].x - bezier[0].x).abs();
                            let min_horizontal = (dx * 0.02).max(0.001);
                            c = match side {
                                TangentSide::Left => c.with_x(c.x.min(endpoint.x - min_horizontal)),
                                TangentSide::Right => {
                                    c.with_x(c.x.max(endpoint.x + min_horizontal))
                                }
                            };
                        }

                        if tangent.weight.is_none() {
                            // Unweighted x is always 1/3 of dx
                            let x = (bezier[3].x - bezier[0].x) * dir / 3.;
                            let relative_c = c - endpoint;
                            c = endpoint + relative_c * (x / relative_c.x);
                        };

                        let (new_slope, new_weight) =
                            slope_weight_from_bezier(bezier[0], bezier[3], endpoint, c, dir);

                        let mut knot = knot.with_tangent_slope(side, new_slope);
                        if tangent.weight.is_some() {
                            knot = knot.with_tangent_weight(side, Some(new_weight));
                        }

                        modified_knot = Some((i, knot));
                    }

                    interact_response.context_menu(|ui| {
                        ui.label("Edit mode");
                        if ui
                            .radio(matches!(tangent.mode, TangentMode::Free), "Free")
                            .clicked()
                        {
                            modified_knot =
                                Some((i, knot.with_tangent_mode(side, TangentMode::Free)));
                            ui.close_menu();
                        }
                        if ui
                            .radio(matches!(tangent.mode, TangentMode::Aligned), "Aligned")
                            .clicked()
                        {
                            modified_knot =
                                Some((i, knot.with_tangent_mode(side, TangentMode::Aligned)));
                            ui.close_menu();
                        }

                        ui.label("Slope:");
                        ui.add(
                            egui::DragValue::from_get_set(|v| match v {
                                Some(v) => {
                                    modified_knot =
                                        Some((i, knot.with_tangent_slope(side, v as f32)));
                                    v
                                }
                                _ => tangent.slope as f64,
                            })
                            .speed(0.001),
                        );

                        let mut weighted = tangent.weight.is_some();
                        if ui.checkbox(&mut weighted, "Weighted").changed() {
                            if weighted && tangent.weight.is_none() {
                                modified_knot =
                                    Some((i, knot.with_tangent_weight(side, Some(1. / 3.))));
                            } else if !weighted {
                                modified_knot = Some((i, knot.with_tangent_weight(side, None)));
                            }
                        };

                        if tangent.weight.is_some() {
                            ui.horizontal(|ui| {
                                ui.label("Weight:");
                                ui.add(
                                    egui::DragValue::from_get_set(|v| match v {
                                        Some(v) => {
                                            modified_knot = Some((
                                                i,
                                                knot.with_tangent_weight(side, Some(v as f32)),
                                            ));
                                            v
                                        }
                                        _ => tangent.weight.unwrap() as f64,
                                    })
                                    .speed(0.001),
                                );
                            });
                        }
                    });

                    painter.add(Shape::dashed_line(
                        &[
                            to_screen.transform_pos(self.curve_to_canvas(knot.position)),
                            point_in_screen,
                        ],
                        Stroke::new(1.0, Color32::GRAY),
                        4.0,
                        2.0,
                    ));

                    painter.add(Shape::circle_filled(
                        point_in_screen,
                        3.0,
                        Color32::LIGHT_GRAY,
                    ));
                };

                // right tangent
                if matches!(knot.interpolation, KnotInterpolation::Cubic) && next_knot.is_some() {
                    tangent_ui(TangentSide::Right);
                }

                // left tangent
                if prev_knot.is_some()
                    && matches!(prev_knot.unwrap().interpolation, KnotInterpolation::Cubic)
                {
                    tangent_ui(TangentSide::Left);
                }
            }

            // Apply modifications
            if let Some((selected_ids, delta)) = move_selected_by {
                if delta.x != 0.0 || delta.y != 0.0 {
                    for knot_id in selected_ids {
                        if let Some(idx) = curve.knots().iter().position(|knot| knot.id == knot_id)
                        {
                            let original = curve.knots()[idx];
                            curve.modify_knot(
                                idx,
                                Knot {
                                    position: original.position + delta,
                                    ..original
                                },
                            );
                        }
                    }
                    changed = true;
                }
            } else if let Some((i, knot)) = modified_knot {
                curve.modify_knot(i, knot);
                changed = true;
            }

            if let Some((selected_ids, interpolation)) = selection_interpolation_change {
                if self.apply_interpolation_to_selection(curve, &selected_ids, interpolation) {
                    changed = true;
                }
            }

            if delete_selected_requested {
                changed |= self.delete_selected_knots(curve);
            } else if delete_selection_from_menu {
                changed |= self.delete_selected_knots(curve);
            } else if let Some(i) = deleted_knot_index {
                curve.delete_knot(i);
                self.selected_knot_ids
                    .retain(|selected_id| curve.knots().iter().any(|knot| knot.id == *selected_id));
                changed = true;
            }

            self.selected_knot_ids
                .retain(|selected_id| curve.knots().iter().any(|knot| knot.id == *selected_id));

            // Sample to visualize and test find_y_given_x
            if let Some(sample) = sample {
                let sample_y = curve.lookup(sample);
                let converted_point = if sample_y.is_finite() {
                    to_screen.transform_pos(self.curve_to_canvas(Vec2::new(sample, sample_y)))
                } else {
                    Pos2::new(f32::NAN, f32::NAN)
                };

                if !sample_y.is_finite() || !converted_point.is_finite() {
                    self.sample_point = None;
                } else {
                    self.sample_point = Some(converted_point);
                    self.bottom_zero_point =
                        to_screen.transform_pos(self.curve_to_canvas(Vec2::new(0.0, 0.0)));
                    self.top_one_point =
                        to_screen.transform_pos(self.curve_to_canvas(Vec2::new(1.0, 1.0)));

                    // sample的那个线线
                    painter.add(Shape::LineSegment {
                        points: [
                            converted_point,
                            to_screen
                                .transform_pos(self.curve_to_canvas(Vec2::new(114.0, sample_y))),
                        ],
                        stroke: Stroke {
                            color: Color32::LIGHT_GREEN,
                            width: 1.0,
                        },
                    });

                    painter.add(Shape::circle_filled(
                        converted_point,
                        3.0,
                        Color32::LIGHT_GREEN,
                    ));
                }
            }

            let min = if self.bipolar { -1.0 } else { 0.0 };
            let points = [
                ([min, min], [min, 1.0]),
                ([min, 1.0], [1.0, 1.0]),
                ([1.0, 1.0], [1.0, min]),
                ([1.0, min], [min, min]),
            ];

            for (p1, p2) in points {
                painter.add(Shape::LineSegment {
                    points: [
                        to_screen.transform_pos(self.curve_to_canvas(Vec2::from_array(p1))),
                        to_screen.transform_pos(self.curve_to_canvas(Vec2::from_array(p2))),
                    ],
                    stroke: Stroke {
                        width: 1.0,
                        color: Color32::from_hex("#FFEAD0").unwrap().gamma_multiply(0.4),
                    },
                });
            }

            if let (Some(start), Some(current)) = (self.marquee_start, self.marquee_current) {
                let selection_rect = Rect::from_two_pos(start, current);
                painter.rect_filled(
                    selection_rect,
                    2.0,
                    Color32::from_hex("#DB9160").unwrap().gamma_multiply(0.15),
                );
                painter.rect_stroke(
                    selection_rect,
                    2.0,
                    Stroke::new(1.0, Color32::from_hex("#DB9160").unwrap()),
                    egui::StrokeKind::Outside,
                );
            }

            self.has_focus = response.has_focus();
        });

        changed
    }

    fn paint_grid(&mut self, painter: &Painter, to_screen: &emath::RectTransform) {
        let label_stride = |pixels_per_step: f32, min_spacing: f32| -> i32 {
            if !pixels_per_step.is_finite() || pixels_per_step <= 0.0 {
                return 1;
            }

            let mut stride = 1;
            while pixels_per_step * (stride as f32) < min_spacing {
                stride *= 2;
            }
            stride
        };

        if self.dbfs_view || self.dbfs_axis_only {
            let x_label = |value: f32, use_dbfs: bool| {
                if use_dbfs {
                    format_dbfs_axis_label(value)
                } else {
                    format!("{value:.1}")
                }
            };
            let y_label = |value: f32| format_dbfs_axis_label(value);

            if self.strict_dbfs_ticks {
                let y_values = self.strict_db_tick_values();
                let min_y = self.offset.y.min(self.offset.y + self.scale.y);
                let max_y = self.offset.y.max(self.offset.y + self.scale.y);
                let visible_y =
                    self.filter_visible_tick_values(y_values, false, min_y, max_y, 18.0);

                for value in visible_y {
                    let line_from = Vec2::new(self.offset.x, value);
                    let line_to = Vec2::new(self.offset.x + self.scale.x, value);
                    painter.add(Shape::LineSegment {
                        points: [
                            to_screen.transform_pos(self.curve_to_canvas(line_from)),
                            to_screen.transform_pos(self.curve_to_canvas(line_to)),
                        ],
                        stroke: Stroke {
                            width: 1.0,
                            color: Color32::from_rgb(42, 42, 42),
                        },
                    });

                    let text_canvas_pos = Pos2::new(5., self.curve_to_canvas(line_from).y);
                    if text_canvas_pos.y < self.editor_size.y - 20.0 {
                        painter.text(
                            to_screen.transform_pos(text_canvas_pos),
                            egui::Align2::LEFT_CENTER,
                            y_label(value),
                            egui::FontId::proportional(10.0),
                            Color32::from_white_alpha(128),
                        );
                    }
                }

                if self.dbfs_x_axis_labels {
                    let x_values = self.strict_db_tick_values();
                    let min_x = self.offset.x.min(self.offset.x + self.scale.x);
                    let max_x = self.offset.x.max(self.offset.x + self.scale.x);
                    let visible_x =
                        self.filter_visible_tick_values(x_values, true, min_x, max_x, 40.0);

                    for value in visible_x {
                        let line_from = Vec2::new(value, self.offset.y);
                        let line_to = Vec2::new(value, self.offset.y + self.scale.y);
                        painter.add(Shape::LineSegment {
                            points: [
                                to_screen.transform_pos(self.curve_to_canvas(line_from)),
                                to_screen.transform_pos(self.curve_to_canvas(line_to)),
                            ],
                            stroke: Stroke {
                                width: 1.0,
                                color: Color32::from_rgb(42, 42, 42),
                            },
                        });

                        let text_canvas_pos =
                            Pos2::new(self.curve_to_canvas(line_from).x, self.editor_size.y - 5.0);
                        painter.text(
                            to_screen.transform_pos(text_canvas_pos),
                            egui::Align2::CENTER_BOTTOM,
                            x_label(value, true),
                            egui::FontId::proportional(10.0),
                            Color32::from_white_alpha(128),
                        );
                    }
                } else if self.grid_step_x > 0.0 {
                    let grid_offset_x = self.offset.x % self.grid_step_x;
                    let grid_x_count = (self.scale.x / self.grid_step_x).ceil() as i32 + 1;
                    let pixels_per_step_x =
                        self.editor_size.x * self.grid_step_x / self.scale.x.max(f32::EPSILON);
                    let x_label_stride = label_stride(pixels_per_step_x, 24.0);
                    for i in 0..grid_x_count {
                        let grid_local_x = (i as f32) * self.grid_step_x - grid_offset_x;

                        let line_from = self.offset + Vec2::new(grid_local_x, 0.0);
                        let line_to = self.offset + Vec2::new(grid_local_x, self.scale.y);

                        painter.add(Shape::LineSegment {
                            points: [
                                to_screen.transform_pos(self.curve_to_canvas(line_from)),
                                to_screen.transform_pos(self.curve_to_canvas(line_to)),
                            ],
                            stroke: Stroke {
                                width: 1.0,
                                color: Color32::from_rgb(42, 42, 42),
                            },
                        });

                        if i % x_label_stride == 0 {
                            painter.text(
                                to_screen.transform_pos(Pos2::new(
                                    self.curve_to_canvas(line_from).x,
                                    self.editor_size.y - 5.,
                                )),
                                egui::Align2::CENTER_BOTTOM,
                                x_label(line_from.x, false),
                                egui::FontId::proportional(10.0),
                                Color32::from_white_alpha(128),
                            );
                        }
                    }
                }

                return;
            }

            if self.grid_step_x > 0.0 {
                let grid_offset_x = self.offset.x % self.grid_step_x;
                let grid_x_count = (self.scale.x / self.grid_step_x).ceil() as i32 + 1;
                let pixels_per_step_x =
                    self.editor_size.x * self.grid_step_x / self.scale.x.max(f32::EPSILON);
                let x_label_stride = label_stride(
                    pixels_per_step_x,
                    if self.dbfs_x_axis_labels { 48.0 } else { 24.0 },
                );
                for i in 0..grid_x_count {
                    let grid_local_x = (i as f32) * self.grid_step_x - grid_offset_x;

                    let line_from = self.offset + Vec2::new(grid_local_x, 0.0);
                    let line_to = self.offset + Vec2::new(grid_local_x, self.scale.y);

                    painter.add(Shape::LineSegment {
                        points: [
                            to_screen.transform_pos(self.curve_to_canvas(line_from)),
                            to_screen.transform_pos(self.curve_to_canvas(line_to)),
                        ],
                        stroke: Stroke {
                            width: 1.0,
                            color: Color32::from_rgb(42, 42, 42),
                        },
                    });

                    if i % x_label_stride == 0 {
                        painter.text(
                            to_screen.transform_pos(Pos2::new(
                                self.curve_to_canvas(line_from).x,
                                self.editor_size.y - 5.,
                            )),
                            egui::Align2::CENTER_BOTTOM,
                            x_label(line_from.x, self.dbfs_x_axis_labels),
                            egui::FontId::proportional(10.0),
                            Color32::from_white_alpha(128),
                        );
                    }
                }
            }

            if self.grid_step_y > 0.0 {
                let grid_offset_y = self.offset.y % self.grid_step_y;
                let grid_y_count = (self.scale.y / self.grid_step_y).ceil() as i32 + 1;
                let pixels_per_step_y =
                    self.editor_size.y * self.grid_step_y / self.scale.y.max(f32::EPSILON);
                let y_label_stride = label_stride(pixels_per_step_y, 28.0);
                for i in 0..grid_y_count {
                    let grid_local_y = (i as f32) * self.grid_step_y - grid_offset_y;

                    let line_from = self.offset + Vec2::new(0.0, grid_local_y);
                    let line_to = self.offset + Vec2::new(self.scale.x, grid_local_y);

                    painter.add(Shape::LineSegment {
                        points: [
                            to_screen.transform_pos(self.curve_to_canvas(line_from)),
                            to_screen.transform_pos(self.curve_to_canvas(line_to)),
                        ],
                        stroke: Stroke {
                            width: 1.0,
                            color: Color32::from_rgb(42, 42, 42),
                        },
                    });

                    let text_canvas_pos = Pos2::new(5., self.curve_to_canvas(line_from).y);
                    if i % y_label_stride == 0 && text_canvas_pos.y < self.editor_size.y - 30. {
                        painter.text(
                            to_screen.transform_pos(text_canvas_pos),
                            egui::Align2::LEFT_CENTER,
                            y_label(line_from.y),
                            egui::FontId::proportional(10.0),
                            Color32::from_white_alpha(128),
                        );
                    }
                }
            }

            return;
        }

        // vertical lines
        if self.grid_step_x > 0.0 {
            let grid_offset_x = self.offset.x % self.grid_step_x;
            let grid_x_count = (self.scale.x / self.grid_step_x).ceil() as i32 + 1;
            let pixels_per_step_x =
                self.editor_size.x * self.grid_step_x / self.scale.x.max(f32::EPSILON);
            let x_label_stride = label_stride(pixels_per_step_x, 24.0);
            for i in 0..grid_x_count {
                let grid_local_x = (i as f32) * self.grid_step_x - grid_offset_x;

                let line_from = self.offset + Vec2::new(grid_local_x, 0.0);
                let line_to = self.offset + Vec2::new(grid_local_x, self.scale.y);

                painter.add(Shape::LineSegment {
                    points: [
                        to_screen.transform_pos(self.curve_to_canvas(line_from)),
                        to_screen.transform_pos(self.curve_to_canvas(line_to)),
                    ],
                    stroke: Stroke {
                        width: 1.0,
                        color: Color32::from_rgb(42, 42, 42),
                    },
                });

                if i % x_label_stride == 0 {
                    painter.text(
                        to_screen.transform_pos(Pos2::new(
                            self.curve_to_canvas(line_from).x,
                            self.editor_size.y - 5.,
                        )),
                        egui::Align2::CENTER_BOTTOM,
                        format!("{:.1}", line_from.x),
                        egui::FontId::proportional(10.0),
                        Color32::from_white_alpha(128),
                    );
                }
            }
        }

        // horizontal lines
        if self.grid_step_y > 0.0 {
            let grid_offset_y = self.offset.y % self.grid_step_y;
            let grid_y_count = (self.scale.y / self.grid_step_y).ceil() as i32 + 1;
            let pixels_per_step_y =
                self.editor_size.y * self.grid_step_y / self.scale.y.max(f32::EPSILON);
            let y_label_stride = label_stride(pixels_per_step_y, 14.0);
            for i in 0..grid_y_count {
                let grid_local_y = (i as f32) * self.grid_step_y - grid_offset_y;

                let line_from = self.offset + Vec2::new(0.0, grid_local_y);
                let line_to = self.offset + Vec2::new(self.scale.x, grid_local_y);

                painter.add(Shape::LineSegment {
                    points: [
                        to_screen.transform_pos(self.curve_to_canvas(line_from)),
                        to_screen.transform_pos(self.curve_to_canvas(line_to)),
                    ],
                    stroke: Stroke {
                        width: 1.0,
                        color: Color32::from_rgb(42, 42, 42),
                    },
                });

                let text_canvas_pos = Pos2::new(5., self.curve_to_canvas(line_from).y);
                if i % y_label_stride == 0 && text_canvas_pos.y < self.editor_size.y - 30. {
                    painter.text(
                        to_screen.transform_pos(text_canvas_pos),
                        egui::Align2::LEFT_CENTER,
                        format!("{:.1}", line_from.y),
                        egui::FontId::proportional(10.0),
                        Color32::from_white_alpha(128),
                    );
                }
            }
        }
    }
}

fn slope_weight_from_bezier(
    c0: Vec2,
    c3: Vec2,
    endpoint: Vec2,
    intermediate: Vec2,
    dir: f32,
) -> (f32, f32) {
    if c3.x == c0.x {
        return (0.0, 1. / 3.);
    }

    let dx = c3.x - c0.x;
    let weight = ((intermediate.x - endpoint.x) * dir / dx).clamp(0.02, 1.0);
    let slope = ((intermediate.y - endpoint.y) * dir / (dx * weight)).clamp(-1.0e4, 1.0e4);
    (slope, weight)
}

fn format_dbfs_axis_label(value: f32) -> String {
    if value.abs() <= 1.0e-6 {
        return "-infdB".to_string();
    }

    let db = dbfs_axis_to_db(value.abs());
    format!("{db:.0}dB")
}

fn dbfs_label_value_to_axis(db: f32) -> f32 {
    if db >= 0.0 {
        1.0
    } else {
        ((db - DBFS_FLOOR_DB) / -DBFS_FLOOR_DB).clamp(0.0, 1.0)
    }
}

fn dbfs_axis_to_db(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * -DBFS_FLOOR_DB) + DBFS_FLOOR_DB
}
