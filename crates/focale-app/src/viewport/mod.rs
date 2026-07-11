//! The colour-managed image viewport (PRD §5).
//!
//! The image is drawn by our own wgpu render pass — never by egui's
//! textured-mesh path — performing working-space → display conversion in
//! the fragment shader (`shader.wgsl`). The shader mirrors the CPU export
//! pathway; all colour matrices are uploaded from `focale_core::color`
//! constants so preview and export share one source of truth.
//!
//! v1 display assumption (docs/architecture.md §7): the surface is sRGB
//! (user-set profile with sRGB default; a `wp_color_management_v1` query
//! slots in here later). The *active rendering gamut* is user-selectable
//! and shown in the status bar.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use focale_core::color::oklab::{OKLAB_M1, OKLAB_M1_INV, OKLAB_M2, OKLAB_M2_INV};
use focale_core::color::{Gamut, Mat3, REC2020_TO_XYZ, REINHARD_WHITE_DEFAULT, rec2020_to_gamut};

/// Number of f32 words in the uniform block (8 mats × 12 + 4 + 4).
const UNIFORM_WORDS: usize = 8 * 12 + 4 + 4;

/// Shared GPU resources, stored in egui-wgpu's callback resources.
pub struct ViewportRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// Current image texture + bind group (recreated on size change).
    image: Option<ImageTexture>,
    /// True when the swapchain format is *not* sRGB, so the shader must
    /// encode.
    shader_encodes_srgb: bool,
}

struct ImageTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    /// Monotonic version of the uploaded pixels.
    version: u64,
}

impl ViewportRenderer {
    /// Creates pipeline and static resources; call once at app start.
    pub fn new(render_state: &egui_wgpu::RenderState) -> Self {
        let device = &render_state.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("focale-viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("focale-viewport"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("focale-viewport"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("focale-viewport"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(render_state.target_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("focale-viewport"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("focale-viewport-uniforms"),
            size: (UNIFORM_WORDS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader_encodes_srgb = !render_state.target_format.is_srgb();
        Self {
            pipeline,
            bind_layout,
            sampler,
            uniforms,
            image: None,
            shader_encodes_srgb,
        }
    }

    /// Uploads working-space pixels (interleaved RGB f32 → RGBA f16) if
    /// `version` is newer than what the GPU holds.
    pub fn upload_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgb_f32: &[f32],
        version: u64,
    ) {
        if let Some(img) = &self.image
            && img.version == version
            && img.width == width
            && img.height == height
        {
            return;
        }
        let needs_new = match &self.image {
            Some(img) => img.width != width || img.height != height,
            None => true,
        };
        if needs_new {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("focale-preview"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("focale-viewport"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms.as_entire_binding(),
                    },
                ],
            });
            self.image = Some(ImageTexture {
                texture,
                bind_group,
                width,
                height,
                version: version.wrapping_sub(1),
            });
        }
        let img = self.image.as_mut().expect("just created");
        if img.version != version {
            let mut halves: Vec<u16> = Vec::with_capacity(width as usize * height as usize * 4);
            for px in rgb_f32.chunks_exact(3) {
                halves.push(half::f16::from_f32(px[0]).to_bits());
                halves.push(half::f16::from_f32(px[1]).to_bits());
                halves.push(half::f16::from_f32(px[2]).to_bits());
                halves.push(half::f16::ONE.to_bits());
            }
            let bytes: Vec<u8> = halves.iter().flat_map(|h| h.to_le_bytes()).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &img.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 8),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            img.version = version;
        }
    }
}

/// Per-frame draw data for the viewport paint callback.
pub struct ViewportCallback {
    /// Active rendering gamut.
    pub gamut: Gamut,
    /// uv scale (xy) and offset (zw): image uv = quad uv * scale + offset.
    pub uv_transform: [f32; 4],
    /// Background grey (linear) for out-of-image area.
    pub background: f32,
}

fn push_mat(words: &mut Vec<f32>, m: Mat3) {
    for row in m.0 {
        words.extend_from_slice(&row);
        words.push(0.0);
    }
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = resources.get_mut::<ViewportRenderer>() else {
            return Vec::new();
        };
        let mut words: Vec<f32> = Vec::with_capacity(UNIFORM_WORDS);
        push_mat(&mut words, REC2020_TO_XYZ);
        push_mat(&mut words, OKLAB_M1);
        push_mat(&mut words, OKLAB_M2);
        push_mat(&mut words, OKLAB_M1_INV);
        push_mat(&mut words, OKLAB_M2_INV);
        push_mat(&mut words, self.gamut.xyz_to_rgb());
        push_mat(&mut words, rec2020_to_gamut(self.gamut));
        // Active gamut → display (v1: display = sRGB).
        let target_to_display = Gamut::Srgb.xyz_to_rgb() * self.gamut.rgb_to_xyz();
        push_mat(&mut words, target_to_display);
        words.extend_from_slice(&self.uv_transform);
        words.extend_from_slice(&[
            REINHARD_WHITE_DEFAULT,
            if renderer.shader_encodes_srgb {
                1.0
            } else {
                0.0
            },
            self.background,
            0.0,
        ]);
        debug_assert_eq!(words.len(), UNIFORM_WORDS);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        queue.write_buffer(&renderer.uniforms, 0, &bytes);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(renderer) = resources.get::<ViewportRenderer>() else {
            return;
        };
        let Some(image) = &renderer.image else {
            return;
        };
        render_pass.set_pipeline(&renderer.pipeline);
        render_pass.set_bind_group(0, &image.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Registers the viewport painter for `rect` this frame.
pub fn paint(ui: &egui::Ui, rect: egui::Rect, callback: ViewportCallback) {
    ui.painter()
        .add(egui_wgpu::Callback::new_paint_callback(rect, callback));
}
