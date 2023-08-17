import { Device } from './device'

export interface ClearOptions {
  color: number[]
}

export class WebGLDevice extends Device {
  public context: WebGL2RenderingContext

  constructor (context: WebGL2RenderingContext) {
    this.context = context
  }

  /**
   * 
   * @param {WebGLTexture} texture 
   * @param {number} unit 
   */
  bindTexture (texture: WebGLTexture, unit: number) {
    this.context.activeTexture()
    this.context.bindTexture()
  }

  unbindTexture (uint: number) {
    this.context.activeTexture()
    this.context.bindTexture()
  }

  bindRenderTarget () {}
  
  bindVertexArray (vertex: WebGlVertexArray) {
    this.context.bindVertexArray()
  }
  
  unbindVertexArray () {
    this.context.bindVertexArray(null)
  }

  setUniform (uniform: WebGLUniformLocation, data: Uniform) {
    
  }

  setRenderState () {}
  bindTexturesAndImages (program: WebGLProgram) {

  }
  setRenderOptions () {}
  resetRenderState () {}
  resetRenderOptions () {}

  clear (options: ClearOptions) {
    let flags = 0

    if (options.color) {
      this.context.colorMask(true, true, true, true)
      this.context.clearColor(color.r, color.g, color.b, color.a)
    }

    if (options.depth) {
      this.context.depthMask(true)
      this.context.clearDepth(options.depth)
    }

    if (options.stencil) {
      this.context.stencilMask()
      this.context.clearStencil(options.stencil)
    }

    if (flags !== 0) {
      this.context.clear(flags)
    }
  }

  preprocess () {}
}

export class WebGLProgramParameters {
  textures: WebGLUniform[]
}

class WebGLProgram {
  public context: WebGL2RenderingContext
  public glProgram: WebGLProgram
  public parameters: WebGLProgram

  delete () {
    this.context.deleteProgram(this.glProgram)
  }
}

class WebGLTexture {

}

class WebGLShader {

  }